#!/usr/bin/env python3
"""Nothing the board or the plan layer runs on may reference pages (ov-269).

Orchestrator pages are an experiment, and the rule that keeps them one is that
nothing depends on them: a page points at cards, lanes and themes by text, and
nothing points back. If a task query joined `board_pages`, or the plan read
called into `pages::`, removing pages would break the board or the plan.

`plan-layer-lint.py` guards the other direction (tasks never name the plan
layer). This guards both the tasks AND the plan layer from naming pages, by
the opposite method: every Rust file under `crates/` is checked, except the
files on PAGE_FILES below, which are the pages' own code and the few registries
that have to list every feature (the capability table, the dispatch match, the
migration list, the CLI's verbs). A new file that names pages fails until it
is added there on purpose, so a join in `tasks.rs` and a call in `plan_read.rs`
are caught without anybody having to remember to guard them.

The other half is `crates/store/src/pages_tests.rs`, which drops both tables
from a populated database and checks every board read and every plan read is
the same bytes, and `crates/daemon/tests/pages_over_the_socket.rs`, which does
it over the wire.

A match is a page table or module name, a page type, or one of the store's
reads and writes called as a method. The word "page" in prose (a terminal's
page, the Plan view's pages in a comment) is not a match.

  ./scripts/page-layer-lint.py              check the tree; exit 1 on a match
  ./scripts/page-layer-lint.py --self-test  plant a reference, see it caught
"""

import pathlib
import re
import shutil
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent

# Where Rust is searched. Anything under these that isn't on PAGE_FILES is
# guarded, so a file added tomorrow is guarded without being listed.
SEARCHED = ["crates"]

# The pages' own code. Each entry
# is a file, or a directory whose whole tree is the pages'. A path that isn't
# there fails the check, so a rename can't leave an allowance for nothing.
PAGE_FILES = [
    # The document: types, validator, schema.
    "crates/core/src/page_doc.rs",
    "crates/core/src/page_doc",
    "crates/core/src/page_schema.rs",
    # The store.
    "crates/store/src/pages.rs",
    "crates/store/src/pages_tests.rs",
    "crates/protocol/src/page_wire_tests.rs",
    # The daemon.
    "crates/daemon/src/rpc_pages.rs",
    "crates/daemon/src/watch/pages.rs",
    "crates/daemon/tests/pages_over_the_socket.rs",
    "crates/daemon/tests/a_page_reads_live_ci.rs",
    # The client: pages as JSON for the phones.
    "crates/client/src/page_json.rs",
    "crates/client/src/page_json_tests.rs",
    "crates/client/src/session/pages.rs",
    "crates/client/src/ffi/page_phone_tests.rs",
    # The CLI.
    "crates/cli/src/page.rs",
    "crates/cli/src/page_text.rs",
    "crates/cli/src/page_tests.rs",
]

# The registries: files that list every feature, and so have to name pages once.
# Each is guarded like any other file except for the lines its pattern matches,
# which are the exact registration: the module line, the dispatch arm, the enum
# variant, the FFI export. A page read planted anywhere else in them fails, so
# `svc.store.list_pages(ws)` in a task route is caught though `rpc.rs` is on this
# list. A path that isn't there fails the check, as PAGE_FILES' do. A comment
# line is never a match.
REGISTRIES = {
    "crates/core/src/lib.rs": r"^\s*pub mod page_(doc|schema);\s*$",
    "crates/core/src/error.rs": r"\bPageRefused\b",
    "crates/store/src/lib.rs": r"^\s*pub mod pages;\s*$",
    "crates/store/src/migrate.rs": r"migration_0025_pages",
    "crates/store/src/testing.rs": r"DROP TABLE page_events; DROP TABLE board_pages;",
    "crates/protocol/src/lib.rs": r"\bBOARD_PAGES\b|^\s*mod page_wire_tests;",
    "crates/daemon/src/lib.rs": r"^\s*pub\(crate\) mod rpc_pages;\s*$",
    "crates/daemon/src/rpc.rs": r"^\s*(\| )?Method::Page(List|Get|Stats|Set|Remove)\b.*(=> Scope::(Read|Control),|\|)?\s*$|crate::rpc_pages::dispatch\(svc, &self\.watcher, req\)\.await",
    "crates/client/src/ffi/route.rs": r"^\s*\| Method::Page(List|Get|Set|Remove|Stats)\b",
    "crates/client/src/ffi.rs": r"Ok\(crate::page_json::pages?_json\(&pages?\)\)|session\.pages?\(id\(\"workspace\"\)\?",
    "crates/client/src/lib.rs": r"^\s*pub mod page_json;\s*$",
    "crates/client/src/session.rs": r"^\s*Payload::PagesChanged\(p\) => Some\(FleetEvent::Pages \{",
    "crates/client/src/session/results.rs": r"^\s*result::Value::(BoardPageList|BoardPage|PageSetResult|PageStatsList)\(_\) =>",
    "crates/cli/src/main.rs": r"event::Payload::PagesChanged\(p\) => event_lines::pages_event_json\(&p\),",
    # main.rs's tests, moved to their own file (ov-455): the event table.
    "crates/cli/src/main_tests.rs": r"\(Payload::PagesChanged\(Default::default\(\)\), \"pages\"\),",
    "crates/cli/src/event_lines.rs": r"pages_event_json|pb::PagesChanged",
    # The CI watch (ov-306) reads the subjects pages name, asked of the pages'
    # own file in one line.
    "crates/daemon/src/ci_watch.rs": r"^\s*for \(workspace, subject\) in crate::rpc_pages::ci_subjects\(svc\) \{\s*$",
}

# What names pages, by meaning rather than by one spelling. Matched over the
# whole file, so a statement split over lines is still one statement.
#   - the tables, the module names and the types;
#   - the store's reads and writes, called as methods;
#   - any `use` that brings the module in, braces and all, and a path through it.
FORBIDDEN = [
    re.compile(
        r"\b(board_pages|page_events|page_doc|page_schema|page_json|rpc_pages|BoardPage\w*|PagesChanged|PageWrite"
        r"|StoredPage|PageRefused|PageSet|PageGet|PageRemove|PageList\w*|PageStats\w*|SlotStats)\b"
    ),
    re.compile(r"\buse\b[^;]*\bpages\b[^;]*;"),
    re.compile(r"(?<![\w.])pages::"),
    re.compile(r"\.(list_pages|get_page|set_page|set_page_at|remove_page|page_stats|page_card_keys)\s*\("),
    re.compile(r"\bMethod::Page\w*"),
    # The client session's page reads, called as methods.
    re.compile(r"\bsession\.pages?\s*\("),
]

# Messages the board and the plan layer put on the wire: their fields never
# name pages.
GUARDED_MESSAGES = [
    "Task", "TaskDetail", "TaskList", "TaskNote", "TaskWorker", "TaskBlock", "Workspace",
    "Plan", "PlanCard", "PlanCoverage", "PlanEvent", "PlanEventList", "PlanChanged",
    "BoardTheme", "BoardThemeView", "Lane", "LaneAgent", "LaneCard", "LaneSpend",
]
PROTO = "proto/farcooler.proto"
PROTO_FORBIDDEN = re.compile(r"\b\w*[Pp]age\w*\b")


def rust_files(root: pathlib.Path):
    for rel in SEARCHED:
        base = root / rel
        if base.exists():
            yield from sorted(p for p in base.rglob("*.rs") if "target" not in p.relative_to(root).parts)


def line_allowed(rel: str, line: str) -> bool:
    if line.strip().startswith("//"):
        return True
    pattern = REGISTRIES.get(rel)
    return bool(pattern and re.search(pattern, line))


def allowed(root: pathlib.Path, path: pathlib.Path) -> bool:
    rel = path.relative_to(root).as_posix()
    return any(rel == entry or rel.startswith(entry + "/") for entry in PAGE_FILES)


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
    """Every (path, line number, text) that names pages, or that guards nothing."""
    hits = []
    for entry in PAGE_FILES:
        if not (root / entry).exists():
            hits.append((entry, 0, "an allowed path is missing: it moved, so the allowance covers nothing. Update PAGE_FILES."))
    for entry in REGISTRIES:
        if not (root / entry).exists():
            hits.append((entry, 0, "a registry is missing: it moved, so its allowance covers nothing. Update REGISTRIES."))
    for path in rust_files(root):
        if allowed(root, path):
            continue
        text = path.read_text()
        rel = path.relative_to(root).as_posix()
        for pattern in FORBIDDEN:
            for m in pattern.finditer(text):
                n = line_of(text, m.start())
                if line_allowed(rel, text.splitlines()[n - 1]):
                    continue
                hits.append((str(path.relative_to(root)), n, text.splitlines()[n - 1].strip()))
    proto = root / PROTO
    if not proto.exists():
        hits.append((PROTO, 0, "the proto is missing: nothing is checking the board's messages."))
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
            "\nThe board and the plan layer must not name pages (ov-269). A page points at cards, lanes "
            "and themes by text; nothing points back, so pages can be removed. Move this into the pages' "
            "own files, or, for a registry that has to list every feature, add the file to PAGE_FILES."
        )
        return 1
    print("page-layer-lint: ok")
    return 0


# One line each registry's pattern allows, for the self-test.
REGISTRY_LINES = {
    "crates/core/src/lib.rs": "pub mod page_doc;",
    "crates/core/src/error.rs": "PageRefused { said: String },",
    "crates/store/src/lib.rs": "pub mod pages;",
    "crates/store/src/migrate.rs": "(crate::pages::migration_0025_pages, Older::Welcome),",
    "crates/store/src/testing.rs": "store.conn().execute_batch(\"DROP TABLE page_events; DROP TABLE board_pages;\").unwrap();",
    "crates/protocol/src/lib.rs": "pub const BOARD_PAGES: &str = \"board_pages\";",
    "crates/daemon/src/lib.rs": "pub(crate) mod rpc_pages;",
    "crates/daemon/src/rpc.rs": "crate::rpc_pages::dispatch(svc, &self.watcher, req).await",
    "crates/client/src/ffi/route.rs": "        | Method::PageList",
    "crates/client/src/ffi.rs": "Ok(crate::page_json::pages_json(&pages))",
    "crates/client/src/lib.rs": "pub mod page_json;",
    "crates/client/src/session.rs": "Payload::PagesChanged(p) => Some(FleetEvent::Pages { workspace: w }),",
    "crates/client/src/session/results.rs": "result::Value::BoardPage(_) => \"board_page\",",
    "crates/cli/src/main.rs": "event::Payload::PagesChanged(p) => event_lines::pages_event_json(&p),",
    "crates/cli/src/main_tests.rs": "        (Payload::PagesChanged(Default::default()), \"pages\"),",
    "crates/cli/src/event_lines.rs": "pub(crate) fn pages_event_json() {}",
    "crates/daemon/src/ci_watch.rs": "    for (workspace, subject) in crate::rpc_pages::ci_subjects(svc) {",
}


def self_test() -> int:
    failures = []
    clean_proto = "".join(f"message {m} {{\n  // The page of a lane's story.\n  bytes worktree_id = 1;\n}}\n" for m in GUARDED_MESSAGES)
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)

        def put(rel, text):
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)

        def clean_tree():
            for child in root.iterdir():
                shutil.rmtree(child) if child.is_dir() else child.unlink()
            for entry in PAGE_FILES:
                if entry.endswith(".rs"):
                    put(entry, "// the pages' own code may name board_pages and use crate::pages;\n")
                else:
                    put(entry + "/mod.rs", "fn ok() {}\n")
            for rel, text in REGISTRY_LINES.items():
                put(rel, text + "\nfn other() {}\n")
            # Files that must stay clean, among them the board's and the plan layer's.
            for rel in [
                "crates/store/src/tasks.rs",
                "crates/store/src/plan.rs",
                "crates/store/src/plan_read.rs",
                "crates/daemon/src/task_ops.rs",
                "crates/daemon/src/rpc_plan.rs",
                "crates/daemon/src/report/gather.rs",
                "crates/cli/src/plan.rs",
                "crates/cli/src/tasks.rs",
            ]:
                put(rel, "// a terminal's page of output\nSELECT 1 FROM tasks;\nuse crate::tasks;\nlet pages_b = 1;\nlet outpage = 1;\n")
            put(PROTO, clean_proto)

        clean_tree()
        flagged = scan(root)
        if flagged:
            failures.append(f"a clean tree was flagged: {flagged}")

        # Each way a reference gets in, in a file that must not have one, by meaning.
        cases = [
            "SELECT * FROM tasks t JOIN board_pages p ON p.workspace_id = t.workspace_id;",
            'conn.execute("INSERT INTO page_events (at) VALUES (1)")',
            'conn.execute("SELECT 1 FROM\n    board_pages WHERE 1")',
            "use crate::pages::StoredPage;",
            "use crate::{pages, tasks};",
            "use crate::{\n    tasks,\n    pages,\n};",
            "let p: pages::StoredPage = x;",
            "farcooler_store::pages::PageWrite {};",
            "let n = self.list_pages(ws)?;",
            "store.set_page(ws, &write, actor)",
            "store.remove_page(ws, slot, actor)",
            "let t = \"board_pages\";",
            "fn f(p: BoardPage) {}",
            "use farcooler_core::page_doc::Page;",
            "match m { Method::PageSet => {} }",
            "Err(DomainError::PageRefused { said })",
            "watcher.announce(PagesChanged {})",
        ]
        for rel in ["crates/store/src/tasks.rs", "crates/store/src/plan_read.rs", "crates/daemon/src/rpc_plan.rs", "crates/cli/src/plan.rs"]:
            for text in cases:
                clean_tree()
                put(rel, text + "\n")
                if not any(h[0] == rel for h in scan(root)):
                    failures.append(f"missed in {rel}: {text!r}")
        # A registry allows its registration lines and nothing else in the file:
        # a page read planted in a task route fails though the file is a registry.
        for rel in REGISTRY_LINES:
            for planted in ["let n = svc.store.list_pages(ws)?;", "use crate::pages::StoredPage;", "let t = \"board_pages\";", "session.page(ws, slot)"]:
                clean_tree()
                put(rel, REGISTRY_LINES[rel] + "\n" + planted + "\n")
                if not any(h[0] == rel for h in scan(root)):
                    failures.append(f"missed in registry {rel}: {planted!r}")
            clean_tree()
            (root / rel).unlink()
            if not any("registry is missing" in h[2] for h in scan(root)):
                failures.append(f"a registry that moved was not flagged: {rel}")
        # A file nobody listed, new or old: guarded by default.
        for rel in ["crates/daemon/src/report/new_file.rs", "crates/store/src/waits.rs", "crates/store/src/brand_new.rs"]:
            clean_tree()
            put(rel, "use crate::{pages};\n")
            if not any(h[0] == rel for h in scan(root)):
                failures.append(f"missed a reference in {rel}")
        # The pages' own files may name pages, and a directory entry covers its tree.
        clean_tree()
        put("crates/core/src/page_doc/parse.rs", "use crate::pages; board_pages\n")
        if scan(root):
            failures.append("the pages' own directory was flagged")
        # An allowance for code that moved must fail, not pass by covering nothing.
        clean_tree()
        (root / "crates/store/src/pages.rs").unlink()
        if not any("missing" in h[2] for h in scan(root)):
            failures.append("an allowed file that moved was not flagged")
        # The messages.
        clean_tree()
        put(PROTO, clean_proto.replace("message TaskNote", "message TaskNoteRenamed"))
        if not any("missing" in h[2] for h in scan(root)):
            failures.append("a guarded message that was renamed was not flagged")
        for name in ["Task", "Lane", "Plan"]:
            clean_tree()
            put(PROTO, clean_proto.replace(f"message {name} {{\n  // The page of a lane's story.\n  bytes worktree_id = 1;", f"message {name} {{\n  bytes worktree_id = 1;\n  bytes page_slot = 2;", 1))
            if not any(f"message {name}" in h[0] and "page_slot" in h[2] for h in scan(root)):
                failures.append(f"missed a page field on message {name}")
    if failures:
        print("\n".join(failures))
        return 1
    print("page-layer-lint self-test: ok")
    return 0


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        sys.exit(self_test())
    if sys.argv[1:]:
        print(__doc__)
        sys.exit(2)
    sys.exit(check())
