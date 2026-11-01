-- A board as the runner built one before ov-872: lanes, trains and worker
-- records, made through the old store calls at 74c7ec8ab (the realistic
-- board in line_migration_tests.rs at that commit), schema 57. What the
-- one-way move to agents, the line and landings is tested on.
PRAGMA foreign_keys=OFF;
BEGIN TRANSACTION;
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
INSERT INTO meta VALUES('schema_version','57');
INSERT INTO meta VALUES('compatible_down_to','21');
CREATE TABLE repository_roots (
            id BLOB PRIMARY KEY,
            host_id BLOB NOT NULL,
            path TEXT NOT NULL UNIQUE,
            created_at INTEGER NOT NULL,
            resource_version INTEGER NOT NULL
        );
INSERT INTO repository_roots VALUES(x'01a19541ab277521a73c92f14e3747e0',x'01a19541ab277521a73c92ddacf53136','/repos/test-01a19541-ab27-7521-a73c-92efba5bd180',0,1);
CREATE TABLE repositories (
            id BLOB PRIMARY KEY,
            host_id BLOB NOT NULL,
            repository_root_id BLOB NOT NULL REFERENCES repository_roots(id),
            display_name TEXT NOT NULL,
            canonical_git_dir TEXT NOT NULL,
            remote_summary TEXT NOT NULL,
            resource_version INTEGER NOT NULL
        , task_key_prefix TEXT NOT NULL DEFAULT '');
INSERT INTO repositories VALUES(x'01a19541ab277521a73c930d1cfc9256',x'01a19541ab277521a73c92ddacf53136',x'01a19541ab277521a73c92f14e3747e0','overnight','/repos/test-01a19541-ab27-7521-a73c-92efba5bd180/.git','',1,'');
CREATE TABLE IF NOT EXISTS "worktrees" (
            id BLOB PRIMARY KEY,
            repository_id BLOB NOT NULL REFERENCES repositories(id),
            branch TEXT NOT NULL,
            worktree_path TEXT NOT NULL,
            hidden INTEGER NOT NULL,
            creation_failed INTEGER NOT NULL,
            resource_version INTEGER NOT NULL
        , is_main_checkout INTEGER NOT NULL DEFAULT 0, worktree_missing INTEGER NOT NULL DEFAULT 0, ordinal INTEGER NOT NULL DEFAULT 0, workspace_id BLOB REFERENCES workspaces(id), claim_source TEXT);
INSERT INTO worktrees VALUES(x'01a19541ab287b40b7bff3234865ad4e',x'01a19541ab277521a73c930d1cfc9256','main','/wt/main',0,0,1,1,0,0,NULL,NULL);
CREATE TABLE terminals (
            id BLOB PRIMARY KEY,
            worktree_id BLOB NOT NULL REFERENCES "worktrees"(id),
            title TEXT NOT NULL,
            command_preset TEXT NOT NULL,
            intent INTEGER NOT NULL,
            runtime_confirmed INTEGER NOT NULL,
            exit_code INTEGER,
            exit_signal INTEGER,
            lease_generation INTEGER NOT NULL,
            epoch INTEGER NOT NULL,
            "columns" INTEGER NOT NULL,
            "rows" INTEGER NOT NULL,
            resource_version INTEGER NOT NULL
        , pane_mode INTEGER NOT NULL DEFAULT 0, agent_session_id TEXT, task_id BLOB REFERENCES tasks(id) ON DELETE SET NULL, workspace_id BLOB REFERENCES workspaces(id), role INTEGER NOT NULL DEFAULT 1, split_of BLOB, split_of_orchestrator INTEGER, web_url TEXT);
INSERT INTO terminals VALUES(x'01a19541ab2978d3ac4aa01ecf9472ec',x'01a19541ab287b40b7bff3234865ad4e','agent','claude',1,0,NULL,NULL,0,0,80,24,3,0,NULL,NULL,x'01a19541ab287b40b7bff3166d3d74f6',2,NULL,NULL,NULL);
INSERT INTO terminals VALUES(x'01a19541ab2978d3ac4aa02745560c8f',x'01a19541ab287b40b7bff3234865ad4e','agent','claude',1,0,NULL,NULL,0,0,80,24,2,0,NULL,NULL,x'01a19541ab287b40b7bff3166d3d74f6',1,NULL,NULL,NULL);
CREATE TABLE idempotency (
            key TEXT NOT NULL,
            client_id BLOB NOT NULL,
            request_hash TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY (key, client_id)
        );
CREATE TABLE review_bases (
            worktree_id BLOB PRIMARY KEY REFERENCES "worktrees"(id) ON DELETE CASCADE,
            base_ref TEXT NOT NULL
        );
CREATE TABLE review_reviewed (
            worktree_id BLOB NOT NULL REFERENCES "worktrees"(id) ON DELETE CASCADE,
            branch TEXT NOT NULL,
            head_commit TEXT NOT NULL,
            worktree_digest TEXT NOT NULL,
            gate_head INTEGER NOT NULL DEFAULT 0,
            gate_index INTEGER NOT NULL DEFAULT 0,
            marked_at INTEGER NOT NULL,
            PRIMARY KEY (worktree_id, branch)
        );
CREATE TABLE review_stack_parents (
            repository_id BLOB NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
            branch TEXT NOT NULL,
            parent_branch TEXT NOT NULL,
            PRIMARY KEY (repository_id, branch)
        );
CREATE TABLE tasks (
            id BLOB PRIMARY KEY NOT NULL,
            -- Deleting a repository cascades here, and from here on to every
            -- note the task ever carried (see task_notes.task_id below). The
            -- notes' append-only triggers do not block this: they guard a
            -- note against being touched while its task is still around, not
            -- against leaving with it. So deleting a repository DOES succeed
            -- and DOES take its whole board with it, decision log included —
            -- the same rule as the ephemeral rows (workspaces, terminals)
            -- this pattern was written for, even though a task's notes are a
            -- decision log the design explicitly wants kept while the task
            -- lives. Left as CASCADE deliberately — changing it is a product
            -- decision, not this migration's to make — but a reader of the
            -- schema should not have to discover the consequence by testing
            -- repository deletion.
            repository_id BLOB NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
            key TEXT NOT NULL,
            title TEXT NOT NULL,
            status TEXT NOT NULL,
            status_since INTEGER NOT NULL,
            intent TEXT NOT NULL DEFAULT '',
            acceptance TEXT NOT NULL DEFAULT '[]',
            constraints TEXT NOT NULL DEFAULT '[]',
            labels TEXT NOT NULL DEFAULT '[]',
            worktree_id BLOB REFERENCES "worktrees"(id) ON DELETE SET NULL,
            created_at INTEGER NOT NULL,
            resource_version INTEGER NOT NULL, former_key TEXT, edited_at INTEGER, workspace_id BLOB REFERENCES workspaces(id), wait_kind TEXT, wait_line TEXT, wait_rank INTEGER, wait_until INTEGER, wait_event TEXT, wait_since INTEGER,
            UNIQUE (repository_id, key)
        );
INSERT INTO tasks VALUES(x'01a19541ab287b40b7bff330fe0a480c',x'01a19541ab277521a73c930d1cfc9256','ov-1','card 0','backlog',1793505471272,'','[]','[]','[]',NULL,1793505471272,1,NULL,NULL,x'01a19541ab287b40b7bff3166d3d74f6',NULL,NULL,NULL,NULL,NULL,NULL);
INSERT INTO tasks VALUES(x'01a19541ab287b40b7bff359871a56ed',x'01a19541ab277521a73c930d1cfc9256','ov-2','card 1','backlog',1793505471272,'','[]','[]','[]',NULL,1793505471272,1,NULL,NULL,x'01a19541ab287b40b7bff3166d3d74f6',NULL,NULL,NULL,NULL,NULL,NULL);
INSERT INTO tasks VALUES(x'01a19541ab287b40b7bff37a8174e296',x'01a19541ab277521a73c930d1cfc9256','ov-3','card 2','in_progress',1793505471278,'','[]','[]','[]',NULL,1793505471272,1,NULL,NULL,x'01a19541ab287b40b7bff3166d3d74f6',NULL,NULL,NULL,NULL,NULL,NULL);
INSERT INTO tasks VALUES(x'01a19541ab287b40b7bff390002b96f2',x'01a19541ab277521a73c930d1cfc9256','ov-4','card 3','backlog',1793505471272,'','[]','[]','[]',NULL,1793505471272,1,NULL,NULL,x'01a19541ab287b40b7bff3166d3d74f6',NULL,NULL,NULL,NULL,NULL,NULL);
INSERT INTO tasks VALUES(x'01a19541ab2978d3ac4a9f3fff685ecf',x'01a19541ab277521a73c930d1cfc9256','ov-5','card 4','backlog',1793505471273,'','[]','[]','[]',NULL,1793505471273,1,NULL,NULL,x'01a19541ab287b40b7bff3166d3d74f6',NULL,NULL,NULL,NULL,NULL,NULL);
INSERT INTO tasks VALUES(x'01a19541ab2978d3ac4a9f55040711a2',x'01a19541ab277521a73c930d1cfc9256','ov-6','card 5','backlog',1793505471273,'','[]','[]','[]',NULL,1793505471273,1,NULL,NULL,x'01a19541ab287b40b7bff3166d3d74f6',NULL,NULL,NULL,NULL,NULL,NULL);
INSERT INTO tasks VALUES(x'01a19541ab2978d3ac4a9f7850b78ed6',x'01a19541ab277521a73c930d1cfc9256','ov-7','card 6','backlog',1793505471273,'','[]','[]','[]',NULL,1793505471273,1,NULL,NULL,x'01a19541ab287b40b7bff3166d3d74f6',NULL,NULL,NULL,NULL,NULL,NULL);
INSERT INTO tasks VALUES(x'01a19541ab2978d3ac4a9f9ea9f53494',x'01a19541ab277521a73c930d1cfc9256','ov-8','card 7','backlog',1793505471273,'','[]','[]','[]',NULL,1793505471273,1,NULL,NULL,x'01a19541ab287b40b7bff3166d3d74f6',NULL,NULL,NULL,NULL,NULL,NULL);
INSERT INTO tasks VALUES(x'01a19541ab2978d3ac4a9fb8ea883706',x'01a19541ab277521a73c930d1cfc9256','ov-9','card 8','backlog',1793505471273,'','[]','[]','[]',NULL,1793505471273,1,NULL,NULL,x'01a19541ab287b40b7bff3166d3d74f6',NULL,NULL,NULL,NULL,NULL,NULL);
INSERT INTO tasks VALUES(x'01a19541ab2978d3ac4a9fd5c7667037',x'01a19541ab277521a73c930d1cfc9256','ov-10','card 9','in_progress',1793505471278,'','[]','[]','[]',NULL,1793505471273,1,NULL,NULL,x'01a19541ab287b40b7bff3166d3d74f6',NULL,NULL,NULL,NULL,NULL,NULL);
INSERT INTO tasks VALUES(x'01a19541ab2978d3ac4a9ff1cf21ff86',x'01a19541ab277521a73c930d1cfc9256','ov-11','card 10','backlog',1793505471273,'','[]','[]','[]',NULL,1793505471273,1,NULL,NULL,x'01a19541ab287b40b7bff3166d3d74f6',NULL,NULL,NULL,NULL,NULL,NULL);
CREATE TABLE task_notes (
            id BLOB PRIMARY KEY NOT NULL,
            -- A task deleted (directly, or by its repository cascading, see
            -- tasks.repository_id above) takes its whole decision log with
            -- it, and succeeds in doing so: the append-only trigger below is
            -- guarded to fire only while the task still exists, so it stops
            -- a note being rewritten or erased one at a time out of a LIVE
            -- log, and gets out of the way of the log leaving as a unit when
            -- its task goes.
            task_id BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            kind TEXT NOT NULL,
            actor TEXT NOT NULL,
            at INTEGER NOT NULL,
            body TEXT NOT NULL,
            extra TEXT NOT NULL DEFAULT '{}',
            supersedes BLOB REFERENCES task_notes(id)
        );
INSERT INTO task_notes VALUES(x'01a19541ab287b40b7bff34cc410de52',x'01a19541ab287b40b7bff330fe0a480c','created','manager',1793505471272,'card 0','{}',NULL);
INSERT INTO task_notes VALUES(x'01a19541ab287b40b7bff361c2abe8fd',x'01a19541ab287b40b7bff359871a56ed','created','manager',1793505471272,'card 1','{}',NULL);
INSERT INTO task_notes VALUES(x'01a19541ab287b40b7bff388c6dd1a08',x'01a19541ab287b40b7bff37a8174e296','created','manager',1793505471272,'card 2','{}',NULL);
INSERT INTO task_notes VALUES(x'01a19541ab2978d3ac4a9f2d3caeb3d0',x'01a19541ab287b40b7bff390002b96f2','created','manager',1793505471273,'card 3','{}',NULL);
INSERT INTO task_notes VALUES(x'01a19541ab2978d3ac4a9f4286b4b98b',x'01a19541ab2978d3ac4a9f3fff685ecf','created','manager',1793505471273,'card 4','{}',NULL);
INSERT INTO task_notes VALUES(x'01a19541ab2978d3ac4a9f69cb9ea3af',x'01a19541ab2978d3ac4a9f55040711a2','created','manager',1793505471273,'card 5','{}',NULL);
INSERT INTO task_notes VALUES(x'01a19541ab2978d3ac4a9f89d9d4f8f9',x'01a19541ab2978d3ac4a9f7850b78ed6','created','manager',1793505471273,'card 6','{}',NULL);
INSERT INTO task_notes VALUES(x'01a19541ab2978d3ac4a9faf4e311ed5',x'01a19541ab2978d3ac4a9f9ea9f53494','created','manager',1793505471273,'card 7','{}',NULL);
INSERT INTO task_notes VALUES(x'01a19541ab2978d3ac4a9fc65e9180d9',x'01a19541ab2978d3ac4a9fb8ea883706','created','manager',1793505471273,'card 8','{}',NULL);
INSERT INTO task_notes VALUES(x'01a19541ab2978d3ac4a9feadb8cd7d5',x'01a19541ab2978d3ac4a9fd5c7667037','created','manager',1793505471273,'card 9','{}',NULL);
INSERT INTO task_notes VALUES(x'01a19541ab2978d3ac4aa0059ba3b58a',x'01a19541ab2978d3ac4a9ff1cf21ff86','created','manager',1793505471273,'card 10','{}',NULL);
INSERT INTO task_notes VALUES(x'01a19541ab2e76d0b017665c1775050c',x'01a19541ab287b40b7bff37a8174e296','worker','manager',1793505471278,'Claude subagent started: Lane web-3: build','{"agent_id":"a-build","event":"started","harness":"claude","linked_by":"orchestrator"}',NULL);
INSERT INTO task_notes VALUES(x'01a19541ab2e76d0b0176669d6511300',x'01a19541ab287b40b7bff37a8174e296','status_change','manager',1793505471278,'moved from backlog to in_progress','{"from":"backlog","to":"in_progress"}',NULL);
INSERT INTO task_notes VALUES(x'01a19541ab2e76d0b017668aa3053c50',x'01a19541ab2978d3ac4a9fd5c7667037','worker','manager',1793505471278,'Claude subagent linked from its description.','{"agent_id":"w-9","event":"started","harness":"claude","linked_by":"description"}',NULL);
INSERT INTO task_notes VALUES(x'01a19541ab2e76d0b01766949c513d59',x'01a19541ab2978d3ac4a9fd5c7667037','status_change','manager',1793505471278,'moved from backlog to in_progress','{"from":"backlog","to":"in_progress"}',NULL);
INSERT INTO task_notes VALUES(x'01a19541ab2e76d0b01766ad55addcae',x'01a19541ab2978d3ac4a9fd5c7667037','worker','manager',1793505471278,'Claude subagent finished.','{"agent_id":"w-9","end_reason":"finished","event":"ended","harness":"claude"}',NULL);
CREATE TABLE task_blocks (
            task_id BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            blocked_by BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            reason TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (task_id, blocked_by)
        );
CREATE TABLE workspaces (
            id BLOB PRIMARY KEY NOT NULL,
            repository_id BLOB NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            task_prefix TEXT NOT NULL,
            is_main INTEGER NOT NULL DEFAULT 0,
            ordinal INTEGER NOT NULL,
            resource_version INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL
        , wake_on_answer INTEGER NOT NULL DEFAULT 1);
INSERT INTO workspaces VALUES(x'01a19541ab287b40b7bff3166d3d74f6',x'01a19541ab277521a73c930d1cfc9256','Main','ov',1,0,1,1793505471272,1);
CREATE TABLE answer_wakes (
            note_id BLOB PRIMARY KEY NOT NULL,
            task_id BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            enqueued_at INTEGER NOT NULL,
            claimed_at INTEGER,
            done_at INTEGER
        , pasted_at INTEGER);
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
            usage TEXT NOT NULL,
            -- `turn`, or `subagent`: a claude subagent's whole run, read from
            -- its own transcript and kept current while it runs. Its spend
            -- counts; turn counts and active time leave it out.
            kind TEXT NOT NULL DEFAULT 'turn'
        );
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
            price_table TEXT, cache_write_1h_tokens INTEGER,
            PRIMARY KEY (turn_id, model)
        );
CREATE TABLE task_workers (
            id BLOB PRIMARY KEY NOT NULL,
            task_id BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            harness TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            session_id TEXT,
            session_cwd TEXT,
            orchestrator_terminal BLOB REFERENCES terminals(id) ON DELETE SET NULL,
            label TEXT NOT NULL DEFAULT '',
            model TEXT NOT NULL DEFAULT '',
            started_at INTEGER NOT NULL,
            ended_at INTEGER,
            end_reason TEXT,
            linked_by TEXT NOT NULL,
            UNIQUE (task_id, harness, agent_id)
        );
INSERT INTO task_workers VALUES(x'01a19541ab2e76d0b0176642dee33066',x'01a19541ab287b40b7bff37a8174e296','claude','a-build','s-1','/wt/main',x'01a19541ab2978d3ac4aa01ecf9472ec','Lane web-3: build','sonnet',1793505471278,NULL,NULL,'orchestrator');
INSERT INTO task_workers VALUES(x'01a19541ab2e76d0b017667bf65acb9e',x'01a19541ab2978d3ac4a9fd5c7667037','claude','w-9','s-1','/wt/main',x'01a19541ab2978d3ac4aa01ecf9472ec','','sonnet',1793505471278,1793505471278,'finished','description');
CREATE TABLE hold_wakes (
            note_id BLOB PRIMARY KEY NOT NULL,
            task_id BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            enqueued_at INTEGER NOT NULL,
            claimed_at INTEGER,
            done_at INTEGER
        , pasted_at INTEGER);
CREATE TABLE board_read_floors (
            workspace_id BLOB PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
            -- Starts at the runner's first look and only ever rises.
            floor_ms INTEGER NOT NULL
        );
CREATE TABLE task_reads (
            task_id BLOB PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
            opened_ms INTEGER NOT NULL
        );
CREATE TABLE board_themes (
            id BLOB PRIMARY KEY NOT NULL,
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            outcome TEXT NOT NULL DEFAULT '',
            story TEXT NOT NULL DEFAULT '',
            next TEXT NOT NULL DEFAULT '',
            owner_ask TEXT NOT NULL DEFAULT '',
            state TEXT NOT NULL,
            ordinal INTEGER NOT NULL,
            -- When `story` was last written; 0 until it first is.
            story_at INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            resource_version INTEGER NOT NULL
        );
INSERT INTO board_themes VALUES(x'01a19541ab2a7e01a141b6ecd99c1b4a',x'01a19541ab287b40b7bff3166d3d74f6','Roles','Agents, not lanes.','','','','active',1,0,1793505471274,1);
CREATE TABLE board_theme_tasks (
            task_id BLOB PRIMARY KEY NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            theme_id BLOB NOT NULL REFERENCES board_themes(id) ON DELETE CASCADE,
            added_at INTEGER NOT NULL
        );
INSERT INTO board_theme_tasks VALUES(x'01a19541ab287b40b7bff330fe0a480c',x'01a19541ab2a7e01a141b6ecd99c1b4a',1793505471274);
INSERT INTO board_theme_tasks VALUES(x'01a19541ab287b40b7bff37a8174e296',x'01a19541ab2a7e01a141b6ecd99c1b4a',1793505471274);
CREATE TABLE lanes (
            id BLOB PRIMARY KEY NOT NULL,
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            state TEXT NOT NULL,
            reason TEXT NOT NULL DEFAULT '',
            -- Set only by `plan set`, and only on a queued lane.
            plan_rank INTEGER,
            worktree_id BLOB REFERENCES worktrees(id) ON DELETE SET NULL,
            worktree_path TEXT NOT NULL DEFAULT '',
            branch TEXT NOT NULL DEFAULT '',
            harness TEXT NOT NULL DEFAULT '',
            model TEXT NOT NULL DEFAULT '',
            train TEXT,
            landed_sha TEXT,
            state_since INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            resource_version INTEGER NOT NULL
        , title TEXT NOT NULL DEFAULT '', held_off INTEGER NOT NULL DEFAULT 0);
INSERT INTO lanes VALUES(x'01a19541ab2a7e01a141b726cf5ffaff',x'01a19541ab287b40b7bff3166d3d74f6','store-1','queued','why store-1',2,NULL,'/wt/store-1','store-1','','',NULL,NULL,1793505471274,1793505471274,2,'',0);
INSERT INTO lanes VALUES(x'01a19541ab2a7e01a141b75a24d1197f',x'01a19541ab287b40b7bff3166d3d74f6','mac-2','queued','why mac-2',1,NULL,'/wt/mac-2','mac-2','','',NULL,NULL,1793505471274,1793505471274,2,'',0);
INSERT INTO lanes VALUES(x'01a19541ab2b7d12a336783eac0593da',x'01a19541ab287b40b7bff3166d3d74f6','web-3','fixing','why web-3',NULL,NULL,'/wt/web-3','web-3','','',NULL,NULL,1793505471275,1793505471275,5,'',0);
INSERT INTO lanes VALUES(x'01a19541ab2b7d12a33678ec089f4a12',x'01a19541ab287b40b7bff3166d3d74f6','old-5','landed','why old-5',NULL,NULL,'/wt/old-5','old-5','','','train-70','abc1234',1793505471277,1793505471275,6,'',0);
INSERT INTO lanes VALUES(x'01a19541ab2d7cd295cb2a70ff5e244b',x'01a19541ab287b40b7bff3166d3d74f6','site-4','landing','why site-4',NULL,NULL,'/wt/site-4','site-4','','','train-71',NULL,1793505471277,1793505471277,4,'',0);
INSERT INTO lanes VALUES(x'01a19541ab2d7cd295cb2b125873b799',x'01a19541ab287b40b7bff3166d3d74f6','gone-6','dropped','why gone-6',NULL,NULL,'/wt/gone-6','gone-6','','',NULL,NULL,1793505471278,1793505471277,2,'',0);
INSERT INTO lanes VALUES(x'01a19541ab2e76d0b0176590e8c1f88d',x'01a19541ab287b40b7bff3166d3d74f6','redo-7','building','why redo-7',NULL,NULL,'/wt/redo-7','redo-7','','',NULL,NULL,1793505471278,1793505471278,1,'',0);
INSERT INTO lanes VALUES(x'01a19541ab2e76d0b01765d20537eaa3',x'01a19541ab287b40b7bff3166d3d74f6','slices-8','queued','why slices-8',NULL,NULL,'/wt/slices-8','slices-8','','',NULL,NULL,1793505471278,1793505471278,1,'',0);
INSERT INTO lanes VALUES(x'01a19541ab2e76d0b0176615f26f3c40',x'01a19541ab287b40b7bff3166d3d74f6','spike-9','building','why spike-9',NULL,NULL,'/wt/spike-9','spike-9','','',NULL,NULL,1793505471278,1793505471278,1,'',0);
CREATE TABLE lane_tasks (
            lane_id BLOB NOT NULL REFERENCES lanes(id) ON DELETE CASCADE,
            task_id BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            -- Empty when the lane works the whole card.
            slice TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (lane_id, task_id, slice)
        );
INSERT INTO lane_tasks VALUES(x'01a19541ab2a7e01a141b726cf5ffaff',x'01a19541ab287b40b7bff330fe0a480c','');
INSERT INTO lane_tasks VALUES(x'01a19541ab2a7e01a141b75a24d1197f',x'01a19541ab287b40b7bff359871a56ed','');
INSERT INTO lane_tasks VALUES(x'01a19541ab2b7d12a336783eac0593da',x'01a19541ab287b40b7bff37a8174e296','');
INSERT INTO lane_tasks VALUES(x'01a19541ab2b7d12a336783eac0593da',x'01a19541ab287b40b7bff390002b96f2','');
INSERT INTO lane_tasks VALUES(x'01a19541ab2b7d12a33678ec089f4a12',x'01a19541ab2978d3ac4a9f55040711a2','');
INSERT INTO lane_tasks VALUES(x'01a19541ab2d7cd295cb2a70ff5e244b',x'01a19541ab2978d3ac4a9f3fff685ecf','');
INSERT INTO lane_tasks VALUES(x'01a19541ab2d7cd295cb2b125873b799',x'01a19541ab2978d3ac4a9f7850b78ed6','');
INSERT INTO lane_tasks VALUES(x'01a19541ab2e76d0b0176590e8c1f88d',x'01a19541ab2978d3ac4a9f7850b78ed6','');
INSERT INTO lane_tasks VALUES(x'01a19541ab2e76d0b01765d20537eaa3',x'01a19541ab2978d3ac4a9f9ea9f53494','a');
INSERT INTO lane_tasks VALUES(x'01a19541ab2e76d0b01765d20537eaa3',x'01a19541ab2978d3ac4a9f9ea9f53494','b');
CREATE TABLE lane_agents (
            lane_id BLOB NOT NULL REFERENCES lanes(id) ON DELETE CASCADE,
            harness TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            role TEXT NOT NULL,
            model TEXT NOT NULL DEFAULT '',
            started_at INTEGER NOT NULL,
            ended_at INTEGER,
            PRIMARY KEY (lane_id, harness, agent_id)
        );
INSERT INTO lane_agents VALUES(x'01a19541ab2b7d12a336783eac0593da','claude','a-build','build','opus',1793505471275,NULL);
INSERT INTO lane_agents VALUES(x'01a19541ab2b7d12a336783eac0593da','claude','a-rev','review','opus',1793505471275,NULL);
INSERT INTO lane_agents VALUES(x'01a19541ab2b7d12a336783eac0593da','codex','a-fix','fix','opus',1793505471275,NULL);
INSERT INTO lane_agents VALUES(x'01a19541ab2b7d12a33678ec089f4a12','claude','b-build','build','opus',1793505471275,NULL);
INSERT INTO lane_agents VALUES(x'01a19541ab2d7cd295cb2a70ff5e244b','claude','pane:01a19541-ab29-78d3-ac4a-a02745560c8f','build','opus',1793505471277,NULL);
INSERT INTO lane_agents VALUES(x'01a19541ab2e76d0b0176590e8c1f88d','claude','c-build','build','opus',1793505471278,NULL);
INSERT INTO lane_agents VALUES(x'01a19541ab2e76d0b0176615f26f3c40','claude','d-spike','build','opus',1793505471278,NULL);
CREATE TABLE plan_events (
            id BLOB PRIMARY KEY NOT NULL,
            at INTEGER NOT NULL,
            actor TEXT NOT NULL,
            theme_id BLOB REFERENCES board_themes(id) ON DELETE CASCADE,
            lane_id BLOB REFERENCES lanes(id) ON DELETE CASCADE,
            kind TEXT NOT NULL,
            body TEXT NOT NULL,
            extra TEXT NOT NULL DEFAULT '{}',
            CHECK ((theme_id IS NULL) <> (lane_id IS NULL))
        );
INSERT INTO plan_events VALUES(x'01a19541ab2a7e01a141b6f0da17cec0',1793505471274,'manager',x'01a19541ab2a7e01a141b6ecd99c1b4a',NULL,'state','Created.','{}');
INSERT INTO plan_events VALUES(x'01a19541ab2a7e01a141b7099b4f706a',1793505471274,'manager',x'01a19541ab2a7e01a141b6ecd99c1b4a',NULL,'cards','Added ov-1.','{}');
INSERT INTO plan_events VALUES(x'01a19541ab2a7e01a141b71c24c1381a',1793505471274,'manager',x'01a19541ab2a7e01a141b6ecd99c1b4a',NULL,'cards','Added ov-3.','{}');
INSERT INTO plan_events VALUES(x'01a19541ab2a7e01a141b7331396a280',1793505471274,'manager',NULL,x'01a19541ab2a7e01a141b726cf5ffaff','state','Queued.','{}');
INSERT INTO plan_events VALUES(x'01a19541ab2a7e01a141b7414fcb83dd',1793505471274,'manager',NULL,x'01a19541ab2a7e01a141b726cf5ffaff','cards','Added ov-1.','{}');
INSERT INTO plan_events VALUES(x'01a19541ab2a7e01a141b766092b1919',1793505471274,'manager',NULL,x'01a19541ab2a7e01a141b75a24d1197f','state','Queued.','{}');
INSERT INTO plan_events VALUES(x'01a19541ab2a7e01a141b7713f5898e9',1793505471274,'manager',NULL,x'01a19541ab2a7e01a141b75a24d1197f','cards','Added ov-2.','{}');
INSERT INTO plan_events VALUES(x'01a19541ab2a7e01a141b78a56942a25',1793505471274,'manager',NULL,x'01a19541ab2a7e01a141b75a24d1197f','plan','In the plan: next up.','{"rank":1}');
INSERT INTO plan_events VALUES(x'01a19541ab2b7d12a3367821e882edea',1793505471275,'manager',NULL,x'01a19541ab2a7e01a141b726cf5ffaff','plan','In the plan: 2nd.','{"rank":2}');
INSERT INTO plan_events VALUES(x'01a19541ab2b7d12a33678449a3a0b5b',1793505471275,'manager',NULL,x'01a19541ab2b7d12a336783eac0593da','state','Started building.','{}');
INSERT INTO plan_events VALUES(x'01a19541ab2b7d12a336785f77a248de',1793505471275,'manager',NULL,x'01a19541ab2b7d12a336783eac0593da','cards','Added ov-3.','{}');
INSERT INTO plan_events VALUES(x'01a19541ab2b7d12a33678692c154028',1793505471275,'manager',NULL,x'01a19541ab2b7d12a336783eac0593da','cards','Added ov-4.','{}');
INSERT INTO plan_events VALUES(x'01a19541ab2b7d12a336787256f1d5df',1793505471275,'manager',NULL,x'01a19541ab2b7d12a336783eac0593da','agent','Started a build agent.','{"agent_id":"a-build"}');
INSERT INTO plan_events VALUES(x'01a19541ab2b7d12a33678870bfc2e71',1793505471275,'manager',NULL,x'01a19541ab2b7d12a336783eac0593da','agent','Started a review agent.','{"agent_id":"a-rev"}');
INSERT INTO plan_events VALUES(x'01a19541ab2b7d12a336789e4094f4ea',1793505471275,'manager',NULL,x'01a19541ab2b7d12a336783eac0593da','state','Moved to review.','{"from":"building","to":"review"}');
INSERT INTO plan_events VALUES(x'01a19541ab2b7d12a33678a1c60cfcb4',1793505471275,'manager',NULL,x'01a19541ab2b7d12a336783eac0593da','state','Started a fix round.','{"from":"review","to":"fixing"}');
INSERT INTO plan_events VALUES(x'01a19541ab2b7d12a33678b9c09b9bb8',1793505471275,'manager',NULL,x'01a19541ab2b7d12a336783eac0593da','agent','Started a fix agent.','{"agent_id":"a-fix"}');
INSERT INTO plan_events VALUES(x'01a19541ab2b7d12a33678d90e935107',1793505471275,'manager',NULL,x'01a19541ab2b7d12a336783eac0593da','budget','budget 50000 tokens','{"from":null,"to":50000}');
INSERT INTO plan_events VALUES(x'01a19541ab2b7d12a33678f25a29ad59',1793505471275,'manager',NULL,x'01a19541ab2b7d12a33678ec089f4a12','state','Started building.','{}');
INSERT INTO plan_events VALUES(x'01a19541ab2b7d12a3367902400e71ec',1793505471275,'manager',NULL,x'01a19541ab2b7d12a33678ec089f4a12','cards','Added ov-6.','{}');
INSERT INTO plan_events VALUES(x'01a19541ab2b7d12a336791214a2a50b',1793505471275,'manager',NULL,x'01a19541ab2b7d12a33678ec089f4a12','agent','Started a build agent.','{"agent_id":"b-build"}');
INSERT INTO plan_events VALUES(x'01a19541ab2b7d12a3367923bd847ca8',1793505471275,'manager',NULL,x'01a19541ab2b7d12a33678ec089f4a12','state','Moved to review.','{"from":"building","to":"review"}');
INSERT INTO plan_events VALUES(x'01a19541ab2c7d60aad9225a3372f55a',1793505471276,'manager',NULL,x'01a19541ab2b7d12a33678ec089f4a12','state','Moved to landing.','{"from":"review","to":"landing"}');
INSERT INTO plan_events VALUES(x'01a19541ab2d7cd295cb2a696a65f858',1793505471277,'manager',NULL,x'01a19541ab2b7d12a33678ec089f4a12','state','Landed with Train 70.','{"from":"landing","to":"landed","train":"train-70"}');
INSERT INTO plan_events VALUES(x'01a19541ab2d7cd295cb2a87c21e331d',1793505471277,'manager',NULL,x'01a19541ab2d7cd295cb2a70ff5e244b','state','Started building.','{}');
INSERT INTO plan_events VALUES(x'01a19541ab2d7cd295cb2a940e7461c0',1793505471277,'manager',NULL,x'01a19541ab2d7cd295cb2a70ff5e244b','cards','Added ov-5.','{}');
INSERT INTO plan_events VALUES(x'01a19541ab2d7cd295cb2aa194cd9969',1793505471277,'manager',NULL,x'01a19541ab2d7cd295cb2a70ff5e244b','agent','Started a build agent.','{"agent_id":"pane:01a19541-ab29-78d3-ac4a-a02745560c8f"}');
INSERT INTO plan_events VALUES(x'01a19541ab2d7cd295cb2ab8ceeda0fe',1793505471277,'manager',NULL,x'01a19541ab2d7cd295cb2a70ff5e244b','state','Moved to review.','{"from":"building","to":"review"}');
INSERT INTO plan_events VALUES(x'01a19541ab2d7cd295cb2b0e698f7e70',1793505471277,'manager',NULL,x'01a19541ab2d7cd295cb2a70ff5e244b','state','Moved to landing.','{"from":"review","to":"landing"}');
INSERT INTO plan_events VALUES(x'01a19541ab2d7cd295cb2b29752f21cf',1793505471277,'manager',NULL,x'01a19541ab2d7cd295cb2b125873b799','state','Queued.','{}');
INSERT INTO plan_events VALUES(x'01a19541ab2d7cd295cb2b3c690022d2',1793505471277,'manager',NULL,x'01a19541ab2d7cd295cb2b125873b799','cards','Added ov-7.','{}');
INSERT INTO plan_events VALUES(x'01a19541ab2e76d0b017658821e4e3b6',1793505471278,'manager',NULL,x'01a19541ab2d7cd295cb2b125873b799','state','Dropped.','{"from":"queued","to":"dropped"}');
INSERT INTO plan_events VALUES(x'01a19541ab2e76d0b01765ab1bb6f291',1793505471278,'manager',NULL,x'01a19541ab2e76d0b0176590e8c1f88d','state','Started building.','{}');
INSERT INTO plan_events VALUES(x'01a19541ab2e76d0b01765ba9da895ca',1793505471278,'manager',NULL,x'01a19541ab2e76d0b0176590e8c1f88d','cards','Added ov-7.','{}');
INSERT INTO plan_events VALUES(x'01a19541ab2e76d0b01765ccd8f5cc5e',1793505471278,'manager',NULL,x'01a19541ab2e76d0b0176590e8c1f88d','agent','Started a build agent.','{"agent_id":"c-build"}');
INSERT INTO plan_events VALUES(x'01a19541ab2e76d0b01765eb90061b26',1793505471278,'manager',NULL,x'01a19541ab2e76d0b01765d20537eaa3','state','Queued.','{}');
INSERT INTO plan_events VALUES(x'01a19541ab2e76d0b01765f2fc7ea14f',1793505471278,'manager',NULL,x'01a19541ab2e76d0b01765d20537eaa3','cards','Added ov-8, a.','{}');
INSERT INTO plan_events VALUES(x'01a19541ab2e76d0b017660b42cb5314',1793505471278,'manager',NULL,x'01a19541ab2e76d0b01765d20537eaa3','cards','Added ov-8, b.','{}');
INSERT INTO plan_events VALUES(x'01a19541ab2e76d0b017662554f915c6',1793505471278,'manager',NULL,x'01a19541ab2e76d0b0176615f26f3c40','state','Started building.','{}');
INSERT INTO plan_events VALUES(x'01a19541ab2e76d0b0176634ac4f7ca6',1793505471278,'manager',NULL,x'01a19541ab2e76d0b0176615f26f3c40','agent','Started a build agent.','{"agent_id":"d-spike"}');
INSERT INTO plan_events VALUES(x'01a19541ab2f7111a6a497f806957791',1793505471279,'manager',x'01a19541ab2a7e01a141b6ecd99c1b4a',NULL,'budget','budget 900000 tokens','{"from":null,"to":900000}');
CREATE TABLE worktree_lfs_pointers (
            worktree_id BLOB NOT NULL REFERENCES worktrees(id) ON DELETE CASCADE,
            path TEXT NOT NULL,
            PRIMARY KEY (worktree_id, path)
        );
CREATE TABLE board_pages (
            id BLOB PRIMARY KEY NOT NULL,
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            slot TEXT NOT NULL,
            -- Copied out of the document so the list doesn't parse every page.
            title TEXT NOT NULL,
            summary TEXT NOT NULL DEFAULT '',
            -- '' or 'theme', and the theme's id as text. No foreign key: a
            -- page must outlive the plan layer.
            anchor_kind TEXT NOT NULL DEFAULT '',
            anchor TEXT NOT NULL DEFAULT '',
            doc_json TEXT NOT NULL,
            schema_v INTEGER NOT NULL,
            bytes INTEGER NOT NULL,
            revision INTEGER NOT NULL,
            ordinal INTEGER NOT NULL,
            actor TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            resource_version INTEGER NOT NULL,
            UNIQUE (workspace_id, slot)
        );
CREATE TABLE page_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            at INTEGER NOT NULL,
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            slot TEXT NOT NULL,
            actor TEXT NOT NULL,
            kind TEXT NOT NULL,
            bytes INTEGER NOT NULL,
            revision INTEGER NOT NULL,
            shape TEXT NOT NULL DEFAULT ''
        );
CREATE TABLE board_rulings (
            id BLOB PRIMARY KEY NOT NULL,
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            -- The short id's number: R-12. Counts up per board, never reused.
            number INTEGER NOT NULL,
            decision TEXT NOT NULL,
            why TEXT NOT NULL,
            reversal TEXT NOT NULL,
            theme_id BLOB REFERENCES board_themes(id) ON DELETE SET NULL,
            state TEXT NOT NULL,
            -- The owner's words, or the orchestrator's, on the last move.
            note TEXT NOT NULL DEFAULT '',
            actor TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            settled_by TEXT,
            settled_at INTEGER,
            resource_version INTEGER NOT NULL, reversed_sha TEXT,
            UNIQUE (workspace_id, number)
        );
CREATE TABLE board_ruling_tasks (
            ruling_id BLOB NOT NULL REFERENCES board_rulings(id) ON DELETE CASCADE,
            task_id BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            PRIMARY KEY (ruling_id, task_id)
        );
CREATE TABLE board_trains (
            id BLOB PRIMARY KEY NOT NULL,
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            -- What it was cut from: `origin/main`, or a SHA.
            base TEXT NOT NULL DEFAULT '',
            pushed_sha TEXT,
            state TEXT NOT NULL,
            state_since INTEGER NOT NULL,
            actor TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            landed_at INTEGER,
            resource_version INTEGER NOT NULL
        , title TEXT NOT NULL DEFAULT '', card_id BLOB REFERENCES tasks(id) ON DELETE SET NULL, agent_harness TEXT NOT NULL DEFAULT '', agent_id TEXT NOT NULL DEFAULT '', agent_model TEXT NOT NULL DEFAULT '', agent_started_at INTEGER, agent_ended_at INTEGER, collecting INTEGER NOT NULL DEFAULT 0);
INSERT INTO board_trains VALUES(x'01a19541ab2b7d12a336793a432f8496',x'01a19541ab287b40b7bff3166d3d74f6','train-1','',NULL,'dropped',1793505471276,'manager',1793505471275,NULL,3,'',NULL,'','','',NULL,NULL,0);
INSERT INTO board_trains VALUES(x'01a19541ab2c7d60aad9223f08a9780d',x'01a19541ab287b40b7bff3166d3d74f6','train-70','origin/main','abc1234','landed',1793505471277,'manager',1793505471276,1793505471277,4,'',x'01a19541ab2978d3ac4a9fb8ea883706','codex','lander-1','gpt',1793505471276,NULL,0);
INSERT INTO board_trains VALUES(x'01a19541ab2c7d60aad922aef2197ae4',x'01a19541ab287b40b7bff3166d3d74f6','train-71','',NULL,'gating',1793505471277,'runner',1793505471276,NULL,2,'',NULL,'','','',NULL,NULL,0);
INSERT INTO board_trains VALUES(x'01a19541ab2d7cd295cb2aeb911eb1db',x'01a19541ab287b40b7bff3166d3d74f6','train-72','',NULL,'integrating',1793505471277,'manager',1793505471277,NULL,1,'',NULL,'','','',NULL,NULL,1);
CREATE TABLE board_ci (
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            subject TEXT NOT NULL,
            -- The commit the runs are for, in full, once a run names it.
            sha TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL,
            url TEXT NOT NULL DEFAULT '',
            -- JSON: [{"name", "state", "url"}], in the order GitHub lists them.
            jobs TEXT NOT NULL DEFAULT '[]',
            -- When a read last worked; 0 if none has.
            fetched_at INTEGER NOT NULL,
            changed_at INTEGER NOT NULL,
            -- When the runner last asked, whether or not GitHub answered.
            asked_at INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (workspace_id, subject)
        );
CREATE TABLE plan_budgets (
            id BLOB PRIMARY KEY NOT NULL,
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            theme_id BLOB REFERENCES board_themes(id) ON DELETE CASCADE,
            lane_id BLOB REFERENCES lanes(id) ON DELETE CASCADE,
            tokens INTEGER NOT NULL CHECK (tokens > 0),
            set_at INTEGER NOT NULL,
            CHECK ((theme_id IS NULL) <> (lane_id IS NULL))
        );
INSERT INTO plan_budgets VALUES(x'01a19541ab2b7d12a33678c814e5eb63',x'01a19541ab287b40b7bff3166d3d74f6',NULL,x'01a19541ab2b7d12a336783eac0593da',50000,1793505471275);
INSERT INTO plan_budgets VALUES(x'01a19541ab2f7111a6a497e59a77e398',x'01a19541ab287b40b7bff3166d3d74f6',x'01a19541ab2a7e01a141b6ecd99c1b4a',NULL,900000,1793505471279);
CREATE TABLE workspace_landing (
            workspace_id BLOB PRIMARY KEY NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            -- `direct` or `pull_requests`; NULL until somebody chooses.
            mode TEXT,
            base TEXT,
            budget_lines INTEGER,
            pr_cost_line INTEGER NOT NULL DEFAULT 0
        , pr_nudge INTEGER NOT NULL DEFAULT 0, pr_nudge_hours INTEGER);
CREATE TABLE message_wakes (
            note_id BLOB PRIMARY KEY NOT NULL,
            task_id BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            to_terminal BLOB,
            enqueued_at INTEGER NOT NULL,
            claimed_at INTEGER,
            pasted_at INTEGER,
            done_at INTEGER
        );
CREATE TABLE compose_holds (
            seq INTEGER PRIMARY KEY AUTOINCREMENT,
            id BLOB NOT NULL UNIQUE,
            terminal_id BLOB NOT NULL REFERENCES terminals(id) ON DELETE CASCADE,
            text TEXT NOT NULL,
            files TEXT NOT NULL,
            enqueued_at INTEGER NOT NULL,
            claimed_at INTEGER
        );
CREATE TABLE compose_hold_images (
            hold_seq INTEGER NOT NULL REFERENCES compose_holds(seq) ON DELETE CASCADE,
            ord INTEGER NOT NULL,
            mime TEXT NOT NULL,
            bytes BLOB NOT NULL,
            PRIMARY KEY (hold_seq, ord)
        );
CREATE TABLE members (
            handle TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            created_at INTEGER NOT NULL
        , account TEXT NULL, email TEXT NULL, role TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('member', 'maintainer')));
CREATE TABLE member_devices (
            client_id TEXT PRIMARY KEY NOT NULL,
            handle TEXT NOT NULL REFERENCES members(handle) ON DELETE CASCADE
        );
CREATE TABLE released_devices (
            client_id TEXT PRIMARY KEY NOT NULL
        );
CREATE TABLE workspace_leads (
            workspace_id BLOB PRIMARY KEY NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            handle TEXT NOT NULL REFERENCES members(handle) ON DELETE CASCADE
        );
CREATE TABLE rep_desks (
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            handle TEXT NOT NULL REFERENCES members(handle) ON DELETE CASCADE,
            task_id BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            PRIMARY KEY (workspace_id, handle)
        );
CREATE TABLE reps (
            terminal_id BLOB PRIMARY KEY NOT NULL REFERENCES terminals(id) ON DELETE CASCADE,
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            handle TEXT NOT NULL REFERENCES members(handle) ON DELETE CASCADE,
            UNIQUE (workspace_id, handle)
        );
CREATE TABLE train_events (
            id BLOB PRIMARY KEY NOT NULL,
            train_id BLOB NOT NULL REFERENCES board_trains(id) ON DELETE CASCADE,
            at INTEGER NOT NULL,
            actor TEXT NOT NULL,
            -- `state`, `lane_on` or `lane_off` (and `held`, which older builds wrote).
            kind TEXT NOT NULL,
            -- The lane a `lane_on` or `lane_off` is about.
            lane_id BLOB,
            extra TEXT NOT NULL DEFAULT '{}'
        );
INSERT INTO train_events VALUES(x'01a19541ab2c7d60aad92200e9d0eb62',x'01a19541ab2b7d12a336793a432f8496',1793505471276,'manager','state',NULL,'{"from":null,"open":true,"to":"integrating"}');
INSERT INTO train_events VALUES(x'01a19541ab2c7d60aad9221661c73faa',x'01a19541ab2b7d12a336793a432f8496',1793505471276,'manager','lane_on',x'01a19541ab2b7d12a33678ec089f4a12','{"from":null}');
INSERT INTO train_events VALUES(x'01a19541ab2c7d60aad9222112d553cf',x'01a19541ab2b7d12a336793a432f8496',1793505471276,'manager','state',NULL,'{"departed":true,"from":"integrating","to":"gating"}');
INSERT INTO train_events VALUES(x'01a19541ab2c7d60aad922416f73bc8f',x'01a19541ab2c7d60aad9223f08a9780d',1793505471276,'manager','state',NULL,'{"from":null,"open":true,"to":"integrating"}');
INSERT INTO train_events VALUES(x'01a19541ab2c7d60aad922646cd82888',x'01a19541ab2c7d60aad9223f08a9780d',1793505471276,'manager','renamed',NULL,'{"from":"train-2","to":"train-70"}');
INSERT INTO train_events VALUES(x'01a19541ab2c7d60aad9227cef438bad',x'01a19541ab2b7d12a336793a432f8496',1793505471276,'manager','lane_off',x'01a19541ab2b7d12a33678ec089f4a12','{"to":"train-70"}');
INSERT INTO train_events VALUES(x'01a19541ab2c7d60aad9228fc4a72c25',x'01a19541ab2b7d12a336793a432f8496',1793505471276,'manager','state',NULL,'{"empty":true,"from":"gating","to":"dropped"}');
INSERT INTO train_events VALUES(x'01a19541ab2c7d60aad9229ebb2f506e',x'01a19541ab2c7d60aad9223f08a9780d',1793505471276,'manager','lane_on',x'01a19541ab2b7d12a33678ec089f4a12','{"from":"train-1"}');
INSERT INTO train_events VALUES(x'01a19541ab2c7d60aad922b1832a427f',x'01a19541ab2c7d60aad922aef2197ae4',1793505471276,'runner','state',NULL,'{"from":null,"open":true,"to":"integrating"}');
INSERT INTO train_events VALUES(x'01a19541ab2c7d60aad922c0f521f26c',x'01a19541ab2c7d60aad9223f08a9780d',1793505471276,'manager','state',NULL,'{"from":"integrating","to":"pushed"}');
INSERT INTO train_events VALUES(x'01a19541ab2d7cd295cb2a562cdcc151',x'01a19541ab2c7d60aad9223f08a9780d',1793505471277,'manager','state',NULL,'{"from":"pushed","to":"landed"}');
INSERT INTO train_events VALUES(x'01a19541ab2d7cd295cb2acf9bfcadd9',x'01a19541ab2c7d60aad922aef2197ae4',1793505471277,'manager','lane_on',x'01a19541ab2d7cd295cb2a70ff5e244b','{"from":null}');
INSERT INTO train_events VALUES(x'01a19541ab2d7cd295cb2ad4a6ba5655',x'01a19541ab2c7d60aad922aef2197ae4',1793505471277,'manager','state',NULL,'{"departed":true,"from":"integrating","to":"gating"}');
INSERT INTO train_events VALUES(x'01a19541ab2d7cd295cb2afa39bb67f3',x'01a19541ab2d7cd295cb2aeb911eb1db',1793505471277,'manager','state',NULL,'{"from":null,"open":true,"to":"integrating"}');
CREATE TABLE subagent_descriptions (
             agent_id TEXT PRIMARY KEY NOT NULL,
             description TEXT NOT NULL,
             recorded_at INTEGER NOT NULL
         );
INSERT INTO subagent_descriptions VALUES('w-9','ov-9 sweep',1793505471279);
INSERT INTO subagent_descriptions VALUES('a-rev','Review web-3',1793505471279);
CREATE TABLE pr_nudges (
            -- `host/owner/repo` on GitHub, lower case.
            repo TEXT NOT NULL,
            pr_number INTEGER NOT NULL,
            -- Unix milliseconds, when the comment was claimed (and so, but for a crash, sent).
            claimed_at INTEGER NOT NULL,
            PRIMARY KEY (repo, pr_number)
        );
CREATE TABLE member_logins (
             handle TEXT PRIMARY KEY NOT NULL REFERENCES members(handle) ON DELETE CASCADE,
             login_id TEXT NOT NULL UNIQUE
         , stopped_at INTEGER, asked_at INTEGER, signed_in_at INTEGER, codex_asked_at INTEGER, cursor_asked_at INTEGER);
CREATE TABLE terminal_logins (
             terminal_id BLOB PRIMARY KEY NOT NULL REFERENCES terminals(id) ON DELETE CASCADE,
             handle TEXT NOT NULL,
             login_id TEXT NOT NULL,
             config_dir TEXT NOT NULL
         , shared INTEGER NOT NULL DEFAULT 0);
CREATE TABLE member_connectors (
             handle TEXT NOT NULL REFERENCES members(handle) ON DELETE CASCADE,
             server TEXT NOT NULL,
             PRIMARY KEY (handle, server)
         );
CREATE TABLE member_login_policy (
             harness TEXT PRIMARY KEY NOT NULL,
             members_own INTEGER NOT NULL,
             set_at INTEGER NOT NULL,
             set_by TEXT NOT NULL
         );
CREATE TABLE device_enrollers (
             client_id TEXT PRIMARY KEY NOT NULL,
             handle TEXT NOT NULL REFERENCES members(handle) ON DELETE CASCADE,
             enrolled_at INTEGER NOT NULL
         );
CREATE TABLE member_board_read_floors (
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            handle TEXT NOT NULL REFERENCES members(handle) ON DELETE CASCADE,
            floor_ms INTEGER NOT NULL,
            PRIMARY KEY (workspace_id, handle)
        );
CREATE TABLE member_task_reads (
            task_id BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            handle TEXT NOT NULL REFERENCES members(handle) ON DELETE CASCADE,
            opened_ms INTEGER NOT NULL,
            PRIMARY KEY (task_id, handle)
        );
CREATE TABLE member_review_reviewed (
            worktree_id BLOB NOT NULL REFERENCES worktrees(id) ON DELETE CASCADE,
            branch TEXT NOT NULL,
            handle TEXT NOT NULL REFERENCES members(handle) ON DELETE CASCADE,
            head_commit TEXT NOT NULL,
            worktree_digest TEXT NOT NULL,
            gate_head INTEGER NOT NULL DEFAULT 0,
            gate_index INTEGER NOT NULL DEFAULT 0,
            marked_at INTEGER NOT NULL,
            PRIMARY KEY (worktree_id, branch, handle)
        );
CREATE TABLE people_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            at INTEGER NOT NULL,
            -- Who made the change: `user` for the owner, `member:<handle>`.
            actor TEXT NOT NULL,
            -- Whom it's about; empty for a charter edit.
            handle TEXT NOT NULL,
            -- The workspace, for a charter edit. No key into workspaces: the
            -- record outlives a workspace deleted.
            workspace_id BLOB,
            kind TEXT NOT NULL,
            detail TEXT NOT NULL DEFAULT '{}'
        );
CREATE TABLE crash_signatures (
             -- The daemon's digest of the signature, lowercase hex.
             signature TEXT PRIMARY KEY NOT NULL,
             -- The signature as first reported, JSON.
             detail TEXT NOT NULL,
             count INTEGER NOT NULL,
             first_seen INTEGER NOT NULL,
             last_seen INTEGER NOT NULL,
             -- The app build of the latest report.
             last_build TEXT NOT NULL,
             -- The card filed for it, if any. No key into tasks: the record
             -- outlives a card deleted.
             task_id BLOB,
             -- Every place it was reported from, JSON: the build and the
             -- app's own frames' offsets, as the daemon spells them.
             places TEXT NOT NULL DEFAULT '[]'
         );
CREATE TABLE compose_waits (
            id BLOB PRIMARY KEY,
            terminal_id BLOB NOT NULL REFERENCES terminals(id) ON DELETE CASCADE,
            wait TEXT NOT NULL,
            kept_at INTEGER NOT NULL
        );
CREATE TABLE pending_teammates (
             client_id TEXT PRIMARY KEY NOT NULL,
             enrolled_at INTEGER NOT NULL
         );
CREATE TABLE revoked_marks (
             client_id TEXT PRIMARY KEY NOT NULL,
             revoked_at INTEGER NOT NULL
         );
CREATE TABLE device_added_by (
             client_id TEXT PRIMARY KEY NOT NULL,
             added_by TEXT NOT NULL,
             added_at INTEGER NOT NULL
         );
CREATE TABLE owner_devices (
             client_id TEXT PRIMARY KEY NOT NULL,
             state TEXT NOT NULL DEFAULT 'confirmed' CHECK (state IN ('confirmed', 'check'))
         );
CREATE TABLE member_access (
             handle TEXT PRIMARY KEY NOT NULL REFERENCES members(handle) ON DELETE CASCADE,
             access TEXT NOT NULL CHECK (access IN ('admin', 'member', 'viewer'))
         );
CREATE TABLE device_caps (
             client_id TEXT PRIMARY KEY NOT NULL,
             cap TEXT NOT NULL CHECK (cap IN ('viewer', 'member'))
         );
CREATE TABLE step_up_keys (
             client_id TEXT PRIMARY KEY NOT NULL,
             alg TEXT NOT NULL,
             pubkey BLOB NOT NULL,
             state TEXT NOT NULL CHECK (state IN ('trusted', 'proposed')),
             registered_at INTEGER NOT NULL,
             sign_count INTEGER NOT NULL DEFAULT 0
         );
CREATE TRIGGER task_notes_forbid_update
        BEFORE UPDATE ON task_notes
        BEGIN
            SELECT RAISE(ABORT, 'task_notes is append-only; supersede instead');
        END;
CREATE TRIGGER task_notes_forbid_delete
        BEFORE DELETE ON task_notes
        WHEN EXISTS (SELECT 1 FROM tasks WHERE id = OLD.task_id)
        BEGIN
            SELECT RAISE(ABORT, 'task_notes is append-only; supersede instead');
        END;
CREATE TRIGGER tasks_need_a_workspace
        BEFORE INSERT ON tasks WHEN NEW.workspace_id IS NULL
        BEGIN
            SELECT RAISE(ABORT, 'a task needs a workspace');
        END;
CREATE TRIGGER tasks_keep_a_workspace
        BEFORE UPDATE OF workspace_id ON tasks WHEN NEW.workspace_id IS NULL
        BEGIN
            SELECT RAISE(ABORT, 'a task needs a workspace');
        END;
CREATE UNIQUE INDEX repositories_one_task_prefix
            ON repositories (task_key_prefix) WHERE task_key_prefix != '';
CREATE INDEX tasks_by_repository ON tasks (repository_id, status);
CREATE INDEX task_notes_by_task ON task_notes (task_id, at);
CREATE INDEX task_notes_by_kind ON task_notes (task_id, kind);
CREATE UNIQUE INDEX worktrees_one_per_path ON worktrees (repository_id, worktree_path);
CREATE INDEX worktrees_by_ordinal ON worktrees (ordinal);
CREATE UNIQUE INDEX workspaces_one_prefix ON workspaces (task_prefix COLLATE NOCASE);
CREATE UNIQUE INDEX workspaces_one_main ON workspaces (repository_id) WHERE is_main = 1;
CREATE INDEX workspaces_by_repository ON workspaces (repository_id, ordinal);
CREATE INDEX tasks_by_workspace ON tasks (workspace_id, status);
CREATE INDEX tasks_by_worktree ON tasks (worktree_id, status);
CREATE INDEX answer_wakes_pending ON answer_wakes (enqueued_at) WHERE done_at IS NULL;
CREATE INDEX agent_turns_by_end ON agent_turns (ended_at);
CREATE INDEX agent_turns_by_task ON agent_turns (task_id, ended_at) WHERE task_id IS NOT NULL;
CREATE INDEX tasks_in_line ON tasks (workspace_id, wait_line, wait_rank) WHERE wait_rank IS NOT NULL;
CREATE INDEX tasks_held_until ON tasks (wait_until) WHERE wait_until IS NOT NULL;
CREATE INDEX task_workers_open ON task_workers (task_id) WHERE ended_at IS NULL;
CREATE INDEX task_workers_by_session ON task_workers (session_id) WHERE session_id IS NOT NULL;
CREATE INDEX hold_wakes_pending ON hold_wakes (enqueued_at) WHERE done_at IS NULL;
CREATE UNIQUE INDEX board_themes_live_name
            ON board_themes (workspace_id, name COLLATE NOCASE) WHERE state <> 'dropped';
CREATE INDEX board_theme_tasks_by_theme ON board_theme_tasks (theme_id);
CREATE UNIQUE INDEX lanes_live_name
            ON lanes (workspace_id, name COLLATE NOCASE) WHERE state NOT IN ('landed', 'dropped');
CREATE INDEX lane_tasks_by_task ON lane_tasks (task_id);
CREATE INDEX plan_events_by_theme ON plan_events (theme_id, at) WHERE theme_id IS NOT NULL;
CREATE INDEX plan_events_by_lane ON plan_events (lane_id, at) WHERE lane_id IS NOT NULL;
CREATE INDEX page_events_by_slot ON page_events (workspace_id, slot, at);
CREATE INDEX board_ruling_tasks_by_task ON board_ruling_tasks (task_id);
CREATE UNIQUE INDEX board_trains_name ON board_trains (workspace_id, name COLLATE NOCASE);
CREATE UNIQUE INDEX plan_budgets_by_theme ON plan_budgets (theme_id) WHERE theme_id IS NOT NULL;
CREATE UNIQUE INDEX plan_budgets_by_lane ON plan_budgets (lane_id) WHERE lane_id IS NOT NULL;
CREATE INDEX message_wakes_pending ON message_wakes (enqueued_at) WHERE done_at IS NULL;
CREATE INDEX compose_holds_by_terminal ON compose_holds (terminal_id, seq);
CREATE INDEX train_events_by_train ON train_events (train_id, at);
CREATE INDEX train_events_by_lane ON train_events (lane_id, at) WHERE lane_id IS NOT NULL;
CREATE UNIQUE INDEX members_by_account ON members(account) WHERE account IS NOT NULL;
CREATE INDEX device_added_by_enroller ON device_added_by(added_by, added_at);
COMMIT;
