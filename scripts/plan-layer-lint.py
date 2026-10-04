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

# Files and directories that must not name the layer.
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
    "crates/daemon/src/task_ops.rs",
    "crates/daemon/src/rpc_board.rs",
    "crates/daemon/src/task_starts.rs",
    "crates/daemon/src/needs_you.rs",
    "crates/daemon/src/board_reads_ops.rs",
    "crates/daemon/src/report",
]

# The layer's own names. `lanes` alone is prose in comments, so it only counts
# after a SQL keyword.
FORBIDDEN = re.compile(
    r"\b(board_themes?|board_theme_tasks|lane_tasks|lane_agents|plan_events|plan_read|rpc_plan"
    r"|BoardTheme\w*|PlanChanged|LaneView|ThemeView)\b"
    r"|\b(FROM|JOIN|INTO|UPDATE|TABLE)\s+lanes\b"
    r"|(farcooler_store|crate)::plan\b",
    re.I,
)

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


def scan(root: pathlib.Path):
    """Every (path, line number, text) that names the layer."""
    hits = []
    for rel in GUARDED:
        for path in files_under(root, rel):
            for n, line in enumerate(path.read_text().splitlines(), 1):
                if FORBIDDEN.search(line):
                    hits.append((str(path.relative_to(root)), n, line.strip()))
    proto = root / PROTO
    if proto.exists():
        source = proto.read_text()
        for name in GUARDED_MESSAGES:
            body = message_body(source, name)
            if body is None:
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
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)

        def put(rel, text):
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)

        put("crates/store/src/tasks.rs", "// a task moving lanes changes both\nSELECT 1 FROM tasks;\n")
        put("crates/daemon/src/report/mod.rs", "fn ok() {}\n")
        put(PROTO, "message Task {\n  // The lane this task is using.\n  bytes worktree_id = 1;\n}\n")
        if scan(root):
            failures.append("a clean tree was flagged")

        cases = [
            ("crates/store/src/tasks.rs", "SELECT * FROM tasks t JOIN lane_tasks l ON l.task_id = t.id;"),
            ("crates/store/src/waits.rs", "conn.execute(\"UPDATE lanes SET reason = ''\")"),
            ("crates/store/src/workers.rs", "use crate::plan::Lane;"),
            ("crates/daemon/src/task_ops.rs", "farcooler_store::plan::prune_moved(&conn);"),
            ("crates/daemon/src/report/gather.rs", "let t = \"board_themes\";"),
        ]
        for rel, text in cases:
            put(rel, text + "\n")
            if not any(h[0] == rel for h in scan(root)):
                failures.append(f"missed a reference in {rel}: {text}")
            (root / rel).unlink()
        put(PROTO, "message Task {\n  bytes worktree_id = 1;\n  bytes lane_id = 2;\n}\n")
        if not any("message Task" in h[0] for h in scan(root)):
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
