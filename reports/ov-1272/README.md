# ov-1272 — Daemon layer inventory and dependency-direction check

Step 1 of [ov-1219's landable sequence](../../.claude/agent/reports/ov-1219/README.md)
(section 5). No code moves. This records which layer each daemon module belongs to,
which layers each may use, and what breaks that today, so steps 2 to 7 (and the
later slices) know their work. The check is `scripts/daemon-layers.py`.

## What is checked

- **Layer of each module:** `scripts/daemon-layers.json`. Every top-level file or
  directory under `crates/daemon/src` belongs to one layer, and a `_tests` file
  sits with the module it tests. A module no layer claims fails the check, so a new
  file must pick one. Ownership follows ov-1219's table, updated for modules added
  since (`engine_steering` and `yielding` are engine, `crash_reports` is board,
  `gh_inert` and `pane_color` are service, `hook_fence` is agents). `watch/answer_wake*`
  and `watch/task_notice*` are delivery; the rest of `watch` is watch.
- **Direction:** from ov-1219's section 2 diagram, taken as allowed direct uses:

  | Layer | May use |
  | --- | --- |
  | git | nothing |
  | access, agents, board, engine | git |
  | service | git, access, agents |
  | delivery | git, agents |
  | watch | git, agents, board, delivery |
  | composition (`rpc*`, `wire`, `main`, `lib`) | anything |

  Anything not listed is forbidden, which includes every implementation layer
  using the composition layer and a layer using a sibling the diagram does not
  give it (engine to board, board to service, access to watch, and so on).
  Contracts and context do not exist yet; when steps 2 and 3 add them, they join
  the table as the layers everything may use.
- **What counts as an edge:** a path into the crate, in code, in a path or in a
  `use` group, resolved to the module that owns it. `crate::a::b` is `a`;
  `super::x` and `self::x` are read against where the file sits in the module tree
  (read from `mod x;` and `#[path]` declarations, and from any inline `mod x { }`
  the path is inside), so `use super::rpc::X` in a top-level `engine.rs` is an edge
  to `rpc`, while the same line in `engine/deep.rs` stays inside `engine`. A file's
  layer is that of the module it sits under, so a `#[path]` test hung beneath
  another module is that module's. A path into `watch::answer_wake*` or
  `watch::task_notice*` is an edge to delivery, written `watch/answer_wake` or
  `watch/task_notice`; other `watch` paths are watch, so delivery to delivery is
  not an edge and a layer that may use watch but not delivery (or the reverse) is
  checked both ways. Comments and strings are not references. This is lexical, like
  ov-1219's `module-edges.json`, not a compiler-resolved graph. It counts test code,
  because a unit test that needs the aggregate `Service` has to move to the
  integration package before its layer can leave the daemon.
- **The allowlist:** `scripts/daemon-layers-allow.txt` holds today's 704 edges,
  one `file -> module` per line. A new edge fails. A listed edge that no longer
  exists also fails, naming the line to delete. In CI, `--allowlist-not-grown
  origin/main` fails when the list holds a line main's does not, so a pull request
  cannot pass by writing a violation down. `--prune` deletes cleared lines and never
  adds one.
- **Where it runs:** the CI `wire` job (a light job in `scripts/lane-gate.py`, which
  reads its steps from `ci.yml`), next to the copy-lint and shared-core steps.

The check was run on today's tree with no allowlist and failed on all 704 edges
before the list was written.

## Violations per layer

704 edges: access 124, agents 65, board 157, delivery 132, engine 115, git 23,
service 36, watch 52. 330 of them are in `_tests` files: those clear when the
test moves (steps 40–45) or no longer needs the aggregate. "Cleared by step" names
the step in ov-1219's table that removes the edge in production code; steps are
numbered as in that table. Where a step is past 7, steps 2 to 7 still leave it listed
and the allowlist keeps it visible.

How steps 2 to 7 clear things:

- **Step 2 (contracts):** the `wire`/`rpc*` edges (caller capabilities, DTOs and
  verdicts the rpc layer and implementations share), caller/people authorization
  verdicts, rep verdicts, board read DTOs and landing-template types.
- **Step 3 (context):** `paths`, `proc_table`, `push`, `test_support`,
  `shared_logins`, `resync`, `stream_end`, `runtime` and the share of the
  `service` aggregate that is only store, tmux or path access. The rest of
  `service` is the terminal slices, 26–31.
- **Steps 4 to 6 (git):** move git code, so they clear nothing in the table
  directly (git may use nothing; its 23 outward edges clear by 2 and 3).
  They make git the bottom layer the others rely on.
- **Step 7 (workflow inbox/runtime owner):** `workflow_runtime`, `workflow_service`,
  `workflow_deciders`, `workflow_notices` and `engine_stuck` as used by service
  and watch.
- **Not in steps 2 to 7:** engine adapters (8–10), access (11–15), agents (16–20),
  board (21–25), delivery (32–36), watch (37–39), test move (40–45).

### git

23 edges in 13 files (6 edges are in test files). May use: nothing below it.

| Uses | Module | Edges | In tests | Cleared by step |
| --- | --- | ---: | ---: | --- |
| access | `people` | 1 | 0 | 2 |
| agents | `hook_install` | 2 | 0 | 16–20 |
| agents | `codex_trust` | 1 | 0 | 16–20 |
| board | `review` | 3 | 0 | 2 |
| board | `rep_gate` | 2 | 1 | 2 |
| board | `pr_timing` | 1 | 0 | 21–25 |
| composition | `rpc` | 1 | 1 | 2 |
| composition | `wire` | 1 | 1 | 2 |
| service | `service` | 4 | 1 | 3, finished by 26–31 |
| service | `paths` | 2 | 1 | 3 |
| service | `test_support` | 2 | 1 | 3 |
| service | `foreground` | 1 | 0 | 26–31 |
| service | `gh_inert` | 1 | 0 | 26–31 |
| service | `proc_table` | 1 | 0 | 3 |

### access

124 edges in 47 files (88 edges are in test files). May use: git.

| Uses | Module | Edges | In tests | Cleared by step |
| --- | --- | ---: | ---: | --- |
| agents | `claude_registry` | 5 | 2 | 16–20 |
| agents | `claude_logins` | 3 | 1 | 16–20 |
| agents | `hook_ingress` | 1 | 1 | 16–20 |
| board | `task_ops` | 2 | 2 | 21–25 |
| board | `needs_you` | 1 | 1 | 21–25 |
| board | `release_sweep` | 1 | 1 | 21–25 |
| board | `review` | 1 | 1 | 2 |
| composition | `rpc` | 15 | 15 | 2 |
| composition | `wire` | 12 | 9 | 2 |
| composition | `rpc_people` | 7 | 7 | 2 |
| delivery | `watch/answer_wake` | 2 | 1 | 32–36 |
| engine | `workflow_runtime` | 1 | 0 | 7 |
| service | `service` | 37 | 16 | 3, finished by 26–31 |
| service | `test_support` | 15 | 15 | 3 |
| service | `proc_table` | 3 | 0 | 3 |
| service | `workspace_home` | 2 | 2 | 26–31 |
| service | `push` | 1 | 0 | 3 |
| service | `shared_logins` | 1 | 0 | 3 |
| service | `workspace_ops` | 1 | 1 | 26–31 |
| watch | `watch` | 13 | 13 | 37–39 |

### agents

65 edges in 22 files (29 edges are in test files). May use: git.

| Uses | Module | Edges | In tests | Cleared by step |
| --- | --- | ---: | ---: | --- |
| access | `people` | 6 | 2 | 2 |
| access | `member_removal` | 2 | 1 | 11–15 |
| access | `writer_gate` | 2 | 0 | 11–15 |
| access | `caller_seat` | 1 | 1 | 2 |
| access | `registry_binding` | 1 | 0 | 11–15 |
| board | `review` | 3 | 1 | 2 |
| board | `claims` | 2 | 1 | 21–25 |
| board | `reps` | 2 | 0 | 2 |
| board | `charter_draft` | 1 | 1 | 21–25 |
| board | `rep_gate` | 1 | 0 | 2 |
| board | `task_link` | 1 | 0 | 21–25 |
| board | `task_ops` | 1 | 0 | 21–25 |
| composition | `wire` | 5 | 3 | 2 |
| composition | `rpc` | 4 | 4 | 2 |
| delivery | `watch/answer_wake` | 1 | 1 | 32–36 |
| service | `service` | 13 | 3 | 3, finished by 26–31 |
| service | `test_support` | 5 | 5 | 3 |
| service | `paths` | 4 | 1 | 3 |
| service | `push` | 2 | 1 | 3 |
| service | `shared_logins` | 2 | 0 | 3 |
| service | `web_pane` | 1 | 0 | 26–31 |
| watch | `watch` | 5 | 4 | 37–39 |

### engine

115 edges in 36 files (40 edges are in test files). May use: git.

| Uses | Module | Edges | In tests | Cleared by step |
| --- | --- | ---: | ---: | --- |
| access | `people` | 8 | 3 | 2 |
| access | `caller_seat` | 5 | 2 | 2 |
| access | `caller_person` | 2 | 1 | 2 |
| access | `cutover_delegate` | 1 | 1 | 11–15 |
| access | `person_access` | 1 | 0 | 11–15 |
| access | `sessions` | 1 | 0 | 11–15 |
| agents | `hook_install` | 8 | 3 | 16–20 |
| agents | `claude_registry` | 2 | 0 | 16–20 |
| agents | `skill_install` | 2 | 0 | 16–20 |
| agents | `claude_logins` | 1 | 0 | 16–20 |
| agents | `log_join` | 1 | 0 | 16–20 |
| board | `review_agent` | 6 | 2 | 21–25 |
| board | `ci_watch` | 4 | 0 | 21–25 |
| board | `task_ops` | 3 | 1 | 21–25 |
| board | `task_starts` | 2 | 0 | 21–25 |
| board | `orchestrator` | 1 | 0 | 21–25 |
| composition | `rpc` | 6 | 3 | 2 |
| composition | `wire` | 3 | 1 | 2 |
| composition | `rpc_agent_stop` | 1 | 0 | 2 |
| composition | `rpc_plan` | 1 | 0 | 2 |
| delivery | `watch/answer_wake` | 2 | 0 | 32–36 |
| delivery | `watch/task_notice` | 1 | 0 | 32–36 |
| service | `service` | 17 | 6 | 3, finished by 26–31 |
| service | `test_support` | 15 | 10 | 3 |
| service | `foreground` | 2 | 0 | 26–31 |
| service | `proc_table` | 2 | 0 | 3 |
| service | `runtime` | 1 | 0 | 3 |
| watch | `watch` | 13 | 5 | 37–39 |
| watch | `watch_view` | 3 | 2 | 37–39 |

### board

157 edges in 56 files (58 edges are in test files). May use: git.

| Uses | Module | Edges | In tests | Cleared by step |
| --- | --- | ---: | ---: | --- |
| access | `people` | 11 | 2 | 2 |
| access | `enrollment` | 3 | 2 | 11–15 |
| access | `caller_seat` | 2 | 0 | 2 |
| access | `allowlist` | 1 | 0 | 11–15 |
| access | `caller_person` | 1 | 0 | 2 |
| access | `cutover_grant` | 1 | 0 | 11–15 |
| access | `sessions` | 1 | 1 | 11–15 |
| agents | `skill_install` | 4 | 1 | 16–20 |
| agents | `claude_login_renewal` | 3 | 1 | 16–20 |
| agents | `claude_logins` | 3 | 2 | 16–20 |
| agents | `hook_ingress` | 2 | 0 | 16–20 |
| agents | `claude_registry` | 1 | 0 | 16–20 |
| agents | `hook_asks` | 1 | 0 | 16–20 |
| agents | `hook_install` | 1 | 0 | 16–20 |
| agents | `session_discovery` | 1 | 1 | 16–20 |
| composition | `wire` | 19 | 5 | 2 |
| composition | `rpc` | 8 | 8 | 2 |
| composition | `rpc_pages` | 1 | 0 | 2 |
| composition | `rpc_people` | 1 | 1 | 2 |
| delivery | `watch/task_notice` | 2 | 0 | 32–36 |
| engine | `engine_stuck` | 3 | 1 | 7 |
| engine | `workflow_deciders` | 1 | 0 | 7 |
| engine | `workflow_drafts` | 1 | 0 | 8–10 |
| service | `service` | 29 | 4 | 3, finished by 26–31 |
| service | `test_support` | 18 | 13 | 3 |
| service | `workspace_home` | 7 | 2 | 26–31 |
| service | `workspace_ops` | 5 | 5 | 26–31 |
| service | `foreground` | 1 | 0 | 26–31 |
| service | `pastes` | 1 | 0 | 26–31 |
| service | `proc_cwd` | 1 | 0 | 26–31 |
| service | `push` | 1 | 0 | 3 |
| service | `shared_logins` | 1 | 0 | 3 |
| watch | `watch` | 21 | 9 | 37–39 |

### service

36 edges in 10 files (4 edges are in test files). May use: access, agents, git.

| Uses | Module | Edges | In tests | Cleared by step |
| --- | --- | ---: | ---: | --- |
| board | `reps` | 3 | 0 | 2 |
| board | `review` | 3 | 0 | 2 |
| board | `lead_change` | 2 | 0 | 21–25 |
| board | `orchestrator` | 2 | 0 | 21–25 |
| board | `plan_glance` | 2 | 0 | 21–25 |
| board | `pr_review` | 2 | 0 | 21–25 |
| board | `by_hand` | 1 | 0 | 21–25 |
| board | `claims` | 1 | 0 | 21–25 |
| board | `landing_read` | 1 | 0 | 2 |
| board | `pr_timing` | 1 | 0 | 21–25 |
| board | `task_ops` | 1 | 0 | 21–25 |
| board | `task_starts` | 1 | 0 | 21–25 |
| board | `worker_seen` | 1 | 0 | 21–25 |
| composition | `wire` | 4 | 2 | 2 |
| composition | `rpc` | 1 | 1 | 2 |
| delivery | `watch/answer_wake` | 2 | 0 | 32–36 |
| engine | `workflow_runtime` | 2 | 0 | 7 |
| engine | `workflow_service` | 1 | 0 | 7 |
| watch | `watch` | 5 | 1 | 37–39 |

### watch

52 edges in 22 files (28 edges are in test files). May use: agents, board, delivery, git.

| Uses | Module | Edges | In tests | Cleared by step |
| --- | --- | ---: | ---: | --- |
| access | `people` | 4 | 2 | 2 |
| access | `registry_binding` | 1 | 0 | 11–15 |
| composition | `wire` | 9 | 5 | 2 |
| composition | `rpc` | 2 | 2 | 2 |
| engine | `engine_stuck` | 1 | 0 | 7 |
| engine | `workflow_deciders` | 1 | 1 | 7 |
| service | `test_support` | 12 | 8 | 3 |
| service | `push` | 7 | 4 | 3 |
| service | `service` | 6 | 4 | 3, finished by 26–31 |
| service | `runtime` | 4 | 1 | 3 |
| service | `resync` | 2 | 1 | 3 |
| service | `foreground` | 1 | 0 | 26–31 |
| service | `layout` | 1 | 0 | 26–31 |
| service | `stream_end` | 1 | 0 | 3 |

### delivery

132 edges in 49 files (77 edges are in test files). May use: agents, git.

| Uses | Module | Edges | In tests | Cleared by step |
| --- | --- | ---: | ---: | --- |
| access | `people` | 7 | 5 | 2 |
| access | `caller_seat` | 5 | 3 | 2 |
| access | `cutover_grant` | 1 | 1 | 11–15 |
| access | `step_up_rpc` | 1 | 1 | 11–15 |
| board | `task_ops` | 5 | 3 | 21–25 |
| board | `needs_you` | 4 | 3 | 21–25 |
| board | `task_link` | 3 | 1 | 21–25 |
| board | `review` | 2 | 1 | 2 |
| board | `review_ops` | 1 | 0 | 2 |
| composition | `wire` | 5 | 3 | 2 |
| composition | `rpc` | 4 | 4 | 2 |
| engine | `engine_steering` | 3 | 1 | 8–10 |
| engine | `effect_runner` | 1 | 1 | 8–10 |
| engine | `workflow_notices` | 1 | 1 | 7 |
| service | `runtime` | 27 | 15 | 3 |
| service | `test_support` | 8 | 6 | 3 |
| service | `pastes` | 4 | 2 | 26–31 |
| service | `service` | 4 | 2 | 3, finished by 26–31 |
| service | `paths` | 3 | 2 | 3 |
| service | `paste_mode` | 2 | 0 | 26–31 |
| service | `proc_table` | 2 | 2 | 3 |
| service | `push` | 1 | 0 | 3 |
| watch | `watch` | 35 | 19 | 37–39 |
| watch | `watch_view` | 3 | 1 | 37–39 |

## Reproduce

```sh
./scripts/daemon-layers.py --self-test
./scripts/daemon-layers.py
./scripts/daemon-layers.py --report
```
