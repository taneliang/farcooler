#!/usr/bin/env python3
"""Nothing the board runs on may reference the plan layer (ov-268).

The plan layer (themes, lanes and the plan) is an experiment, and the rule
that keeps it one is that tasks never depend on it: new tables point at
tasks, and nothing points back. If a task query joined `lane_tasks`, or
`task_ops` called into the layer, removing the layer would break the board.

This checks the half a drill can't see by running: that the code which reads
and writes tasks never NAMES the layer. The other half is
`crates/store/src/plan_tests.rs`, which drops the layer's six tables from a
populated database and checks every board read is the same bytes.

Guarded: the store files and daemon files that serve the board, and the
`Task`-shaped proto messages. A match is the layer's table or module name, or
one of its names in SQL, not the word "lane" in prose (a worktree has always
been called a lane in comments).

  ./scripts/plan-layer-lint.py              check the tree; exit 1 on a match
  ./scripts/plan-layer-lint.py --self-test  plant a reference, see it caught
"""

import pathlib
import re
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent

# Files and directories that must not name the layer. A path that isn't there
# fails the check: a refactor that moves one of these (task_ops into a
# directory, tasks.rs split under the size budget) would otherwise leave the
# guard checking nothing and green. Update this list with the move.
GUARDED = [
    "crates/store/src/tasks.rs",
    "crates/store/src/waits.rs",
    "crates/store/src/workers.rs",
    "crates/store/src/workspaces.rs",
    "crates/store/src/wakes.rs",
    "crates/store/src/store.rs",
    "crates/store/src/models.rs",
    "crates/store/src/usage.rs",
    "crates/store/src/board_reads.rs",
    "crates/store/src/review.rs",
    "crates/store/src/backup.rs",
    "crates/daemon/src/task_ops.rs",
    "crates/daemon/src/rpc_board.rs",
    "crates/daemon/src/task_starts.rs",
    "crates/daemon/src/needs_you.rs",
    "crates/daemon/src/board_reads_ops.rs",
    "crates/daemon/src/usage.rs",
    "crates/daemon/src/watch/task_notice.rs",
    "crates/daemon/src/report",
    # The `task` verbs are unchanged, so their CLI never names the layer.
    "crates/cli/src/tasks.rs",
]

# The layer, by meaning rather than by one spelling. Matched over the whole
# file, so a statement split over lines is still one statement.
#   - its tables, and its names in SQL (`lanes` alone is prose in comments, so
#     it only counts after a SQL keyword, however the line breaks);
#   - its modules and types, however they are imported: `crate::plan`,
#     `use crate::{plan, tasks}`, `plan::Lane`, `use farcooler_store::plan::*`;
#   - its reads, called as methods (`store.plan(..)`, `store.lane(..)`);
#   - its columns and ids.
FORBIDDEN = [
    re.compile(
        r"\b(board_themes?|board_theme_tasks|lane_tasks|lane_agents|plan_events|plan_read|rpc_plan"
        r"|plan_layer|plan_rank|BoardTheme\w*|PlanChanged|LaneView|ThemeView|LaneState|LaneCard"
        r"|LaneAgent\w*|AgentRecord|lane_id|theme_id)\b"
    ),
    re.compile(r"\b(FROM|JOIN|INTO|UPDATE|TABLE)\s+lanes\b", re.I),
    # Any `use` that brings the layer in, braces and all.
    re.compile(r"\buse\b[^;]*\b(plan|plan_read|rpc_plan)\b[^;]*;"),
    # A path through the module: `crate::plan::..`, `farcooler_store::plan::..`, `plan::Lane`.
    re.compile(r"(?<![\w.])plan::"),
    # The layer's reads and writes, called on a store or a service.
    re.compile(r"\.(plan|board_theme|create_theme|update_theme|theme_cards|create_lane|update_lane"
               r"|lane_cards|record_lane_agent|set_plan|plan_events)\s*\("),
]

# Task-shaped proto messages: their fields never name the layer.
GUARDED_MESSAGES = ["Task", "TaskDetail", "TaskList", "TaskNote", "TaskWorker", "TaskBlock", "Workspace"]
PROTO = "proto/farcooler.proto"
PROTO_FORBIDDEN = re.compile(r"\b(lane_\w+|\w*lane_id|board_theme\w*|theme_id|plan_rank|plan_event\w*)\b|\bBoardTheme\b|\bLane\b|\bPlan\b")


def files_under(root: pathlib.Path, rel: str):
    path = root / rel
    if path.is_dir():
        yield from sorted(p for p in path.rglob("*.rs"))
    elif path.exists():
        yield path


def message_body(source: str, name: str):
    """The text of `message <name> { ... }`, found by brace matching."""
    m = re.search(rf"^message {name}\s*\{{", source, re.M)
    if not m:
        return None
    depth, i = 0, m.end() - 1
    for j in range(i, len(source)):
        depth += {"{": 1, "}": -1}.get(source[j], 0)
        if depth == 0:
            return source[m.start() : j + 1]
    return None


def line_of(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


def scan(root: pathlib.Path):
    """Every (path, line number, text) that names the layer, or that guards nothing."""
    hits = []
    for rel in GUARDED:
        found = list(files_under(root, rel))
        if not found:
            hits.append((rel, 0, "guarded path is missing: it moved, so nothing is checking it. Update GUARDED."))
        for path in found:
            text = path.read_text()
            for pattern in FORBIDDEN:
                for m in pattern.finditer(text):
                    n = line_of(text, m.start())
                    hits.append((str(path.relative_to(root)), n, text.splitlines()[n - 1].strip()))
    proto = root / PROTO
    if not proto.exists():
        hits.append((PROTO, 0, "the proto is missing: nothing is checking the Task messages."))
        return hits
    source = proto.read_text()
    for name in GUARDED_MESSAGES:
        body = message_body(source, name)
        if body is None:
            hits.append((f"{PROTO}: message {name}", 0, "guarded message is missing: it was renamed, so nothing is checking it."))
            continue
        for n, line in enumerate(body.splitlines(), 1):
            code = line.split("//")[0]
            if PROTO_FORBIDDEN.search(code):
                hits.append((f"{PROTO}: message {name}", n, line.strip()))
    return hits


def check() -> int:
    hits = scan(ROOT)
    for path, n, line in hits:
        print(f"{path}:{n}: {line}")
    if hits:
        print(
            "\nThe board must not name the plan layer (ov-268). The layer points at tasks; "
            "tasks never point at it, so it can be removed. Move this into plan.rs / plan_read.rs "
            "or the layer's own daemon and CLI files."
        )
        return 1
    print("plan-layer-lint: ok")
    return 0


def self_test() -> int:
    failures = []
    clean_proto = "".join(f"message {m} {{\n  // The lane this task is using.\n  bytes worktree_id = 1;\n}}\n" for m in GUARDED_MESSAGES)
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)

        def put(rel, text):
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)

        def clean_tree():
            for rel in GUARDED:
                if rel.endswith(".rs"):
                    put(rel, "// a task moving lanes changes both\nSELECT 1 FROM tasks;\nuse crate::tasks;\nlet plan_b = 1;\n")
                else:
                    put(rel + "/mod.rs", "fn ok() {}\n")
            put(PROTO, clean_proto)

        clean_tree()
        flagged = scan(root)
        if flagged:
            failures.append(f"a clean tree was flagged: {flagged}")

        # Each way a reference gets in, in a guarded file, by meaning.
        in_file = "crates/store/src/waits.rs"
        cases = [
            "SELECT * FROM tasks t JOIN lane_tasks l ON l.task_id = t.id;",
            'conn.execute("UPDATE lanes SET reason = \'\'")',
            'conn.execute("SELECT 1 FROM\n    lanes WHERE 1")',
            "use crate::plan::Lane;",
            "use crate::{plan, tasks};",
            "use crate::{\n    tasks,\n    plan,\n};",
            "let l: plan::Lane = x;",
            "farcooler_store::plan::prune_moved(&conn);",
            "let n = self.plan(ws, 0)?;",
            "store.set_plan(ws, &lanes, actor)",
            'conn.execute("UPDATE tasks SET plan_rank = 1")',
            "let t = \"board_themes\";",
            "fn f(s: LaneState) {}",
        ]
        for text in cases:
            clean_tree()
            put(in_file, text + "\n")
            if not any(h[0] == in_file for h in scan(root)):
                failures.append(f"missed in {in_file}: {text!r}")
        # Files the guard was too narrow for, and one inside a directory guard.
        for rel in ["crates/cli/src/tasks.rs", "crates/daemon/src/usage.rs", "crates/daemon/src/watch/task_notice.rs",
                    "crates/store/src/backup.rs", "crates/daemon/src/report/gather.rs"]:
            clean_tree()
            put(rel, "use crate::{plan};\n")
            if not any(h[0] == rel for h in scan(root)):
                failures.append(f"missed a reference in {rel}")
        # The code it guards moving must fail, not pass by checking nothing.
        clean_tree()
        (root / "crates/daemon/src/task_ops.rs").unlink()
        if not any("missing" in h[2] for h in scan(root)):
            failures.append("a guarded file that moved was not flagged")
        clean_tree()
        put(PROTO, clean_proto.replace("message TaskNote", "message TaskNoteRenamed"))
        if not any("missing" in h[2] for h in scan(root)):
            failures.append("a guarded message that was renamed was not flagged")
        clean_tree()
        put(PROTO, clean_proto + "message TaskExtra {}\n")
        put(PROTO, clean_proto.replace("bytes worktree_id = 1;", "bytes worktree_id = 1;\n  bytes lane_id = 2;", 1))
        if not any("message Task" in h[0] and "lane_id" in h[2] for h in scan(root)):
            failures.append("missed a lane field on message Task")
    if failures:
        print("\n".join(failures))
        return 1
    print("plan-layer-lint self-test: ok")
    return 0


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        sys.exit(self_test())
    if sys.argv[1:]:
        print(__doc__)
        sys.exit(2)
    sys.exit(check())
