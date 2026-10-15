#!/usr/bin/env python3
"""Fail on a migration number or a rate-limit namespace id two lanes could both claim (ov-553).

Two lanes that reserve the same number each pass their own tests, and the
second to merge breaks the deploy (or, for a store migration, the database).
Every security and store review used to check by hand that a number wasn't
taken on another branch or on main; after a rebase onto main the collision is
visible in the tree, so this reads the tree:

  relay / any `migrations` folder of `NNNN_name.sql` files
      No two share a number, none is named off the pattern, and the numbers run
      1, 2, 3 ... with no gap: a lane that takes N+2 while main is at N has
      reserved a number nobody has, and the next lane takes N+1 and lands
      "before" it. Wrangler applies them in name order.
  store / crates/store/src/migrate.rs
      The labels in `MIGRATIONS` (`migration_0042_...`) are unique and rise in
      list order. The list's position is the schema version, so the labels may
      skip numbers (they do, for numbers other lanes reserved and never landed),
      but never repeat or go backwards. A migration function defined anywhere in
      `crates/store/src` carries a label no other has.
  Cloudflare rate limits / services/*/wrangler.toml
      A `namespace_id` is shared across the whole account, not per worker, so the
      ids in the relay's and the site's files are unique across both files.

  ./scripts/migration-lint.py
  ./scripts/migration-lint.py --self-test
"""

import os
import re
import shutil
import sys
import tempfile

SKIP = {".git", ".claude", "target", "node_modules", ".build", "DerivedData", ".wrangler"}
SQL = re.compile(r"^(\d{4})_[A-Za-z0-9_]+\.sql$")
LABEL = re.compile(r"\bmigration_(\d{4})_[a-z0-9_]+")
# The list holds 42 today and only grows; fewer means the parse broke.
MIN_STORE_MIGRATIONS = 40
NAMESPACE = re.compile(r'^\s*namespace_id\s*=\s*"([^"]*)"', re.M)


def walk(root):
    for folder, dirs, files in os.walk(root):
        dirs[:] = sorted(d for d in dirs if d not in SKIP)
        yield folder, sorted(files)


def sql_folders(root):
    """Every `migrations` folder under `root` that holds a `.sql` file."""
    for folder, files in walk(root):
        if os.path.basename(folder) == "migrations" and any(f.endswith(".sql") for f in files):
            yield folder, [f for f in files if f.endswith(".sql")]


def check_sql(root):
    problems = []
    for folder, names in sql_folders(root):
        where = os.path.relpath(folder, root)
        numbers = {}
        for name in names:
            m = SQL.match(name)
            if not m:
                problems.append(f"{where}/{name}: a migration is named `NNNN_what_it_does.sql`")
                continue
            numbers.setdefault(int(m.group(1)), []).append(name)
        for number, same in sorted(numbers.items()):
            if len(same) > 1:
                problems.append(f"{where}: {' and '.join(same)} share number {number}; the second to merge breaks the deploy. Take the next free number after a rebase onto main")
        if numbers:
            missing = [n for n in range(1, max(numbers) + 1) if n not in numbers]
            if missing:
                problems.append(f"{where}: no migration numbered {', '.join(f'{n:04d}' for n in missing[:5])}: the numbers run without a gap, so a new one is main's last plus one")
    return problems


def read(path):
    try:
        with open(path, encoding="utf-8") as f:
            return f.read()
    except OSError:
        return ""


def blank_comments(text):
    text = re.sub(r"/\*.*?\*/", lambda m: re.sub(r"[^\n]", " ", m.group(0)), text, flags=re.S)
    return re.sub(r"//[^\n]*", "", text)


def check_store(root):
    problems = []
    store = os.path.join("crates", "store", "src")
    text = blank_comments(read(os.path.join(root, store, "migrate.rs")))
    start = text.find("MIGRATIONS")
    listed = [int(n) for n in LABEL.findall(text[start:text.find("];", start)])] if start >= 0 else []
    if len(listed) < MIN_STORE_MIGRATIONS:
        problems.append(f"{store}/migrate.rs: found {len(listed)} migrations in MIGRATIONS, fewer than {MIN_STORE_MIGRATIONS}; the list's format changed, or this lint no longer reads it")
    for before, after in zip(listed, listed[1:]):
        if after == before:
            problems.append(f"{store}/migrate.rs: migration {after:04d} is listed twice")
        elif after < before:
            problems.append(f"{store}/migrate.rs: migration {after:04d} is listed after {before:04d}; labels rise in list order")
    defined = {}
    for folder, files in walk(os.path.join(root, store)):
        for name in files:
            if name.endswith(".rs"):
                path = os.path.join(folder, name)
                for m in re.finditer(r"\bfn\s+migration_(\d{4})_[a-z0-9_]+", blank_comments(read(path))):
                    defined.setdefault(int(m.group(1)), []).append(os.path.relpath(path, root))
    for number, places in sorted(defined.items()):
        if len(places) > 1:
            problems.append(f"{number:04d}: store migration defined in {', '.join(places)}; each label is used once")
    return problems


def check_namespaces(root):
    seen = {}
    problems = []
    services = os.path.join(root, "services")
    for folder, files in walk(services):
        if "wrangler.toml" not in files:
            continue
        path = os.path.relpath(os.path.join(folder, "wrangler.toml"), root)
        text = re.sub(r"(?m)^\s*#.*$", "", read(os.path.join(folder, "wrangler.toml")))
        for ident in NAMESPACE.findall(text):
            if ident in seen:
                problems.append(f"{path}: rate-limit namespace_id \"{ident}\" is also in {seen[ident]}; they are shared across the whole Cloudflare account")
            else:
                seen[ident] = path
    return problems


def lint(root):
    return check_sql(root) + check_store(root) + check_namespaces(root)


def run(root):
    problems = lint(root)
    for line in problems:
        print(f"{line} (ov-553)", file=sys.stderr)
    return 1 if problems else 0


def put(root, rel, body=""):
    path = os.path.join(root, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(body)


def self_test():
    """Red on each planted collision, green on the tree they were planted in."""
    here = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
    scratch = tempfile.mkdtemp(prefix="fc-migration-lint-")
    failures = []
    try:
        def fresh():
            """A copy of the real migration names, store list and wrangler files."""
            tree = tempfile.mkdtemp(dir=scratch)
            for folder, names in sql_folders(here):
                for name in names:
                    put(tree, os.path.join(os.path.relpath(folder, here), name))
            store = os.path.join("crates", "store", "src")
            rels = [os.path.join(store, n) for n in os.listdir(os.path.join(here, store)) if n.endswith(".rs")]
            for rel in rels + ["services/relay/wrangler.toml", "services/site/wrangler.toml"]:
                put(tree, rel, read(os.path.join(here, rel)))
            return tree

        def expect(name, tree, red, needle=None):
            found = lint(tree)
            if bool(found) != red or (red and needle and not any(needle in p for p in found)):
                failures.append(f"{name}: wanted {'a problem naming ' + repr(needle) if red else 'none'}, got {found}")

        real = fresh()
        if not any(True for _ in sql_folders(real)):
            failures.append("no relay migrations folder found in the real tree")
        # The relay's highest migration number, read the way lint reads the folder.
        relay = os.path.join("services", "relay", "migrations")
        relay_names = next((names for folder, names in sql_folders(here) if os.path.relpath(folder, here) == relay), [])
        last = max((int(SQL.match(n).group(1)) for n in relay_names if SQL.match(n)), default=0)
        if not last:
            failures.append(f"no numbered migrations found in {relay} of the real tree")
        expect("the real tree, copied", real, False)

        tree = fresh()
        put(tree, os.path.join(relay, f"{last:04d}_something_else.sql"))
        expect(f"a second migration numbered {last:04d}", tree, True, f"share number {last}")

        tree = fresh()
        put(tree, os.path.join(relay, f"{last + 2:04d}_gap.sql"))
        expect(f"a gap before {last + 2:04d}", tree, True, f"no migration numbered {last + 1:04d}")

        tree = fresh()
        put(tree, os.path.join(relay, f"{last + 1:04d}_next.sql"))
        expect("the next number", tree, False)

        tree = fresh()
        put(tree, "services/relay/migrations/25_next.sql")
        expect("a file off the pattern", tree, True, "NNNN_what_it_does.sql")

        text = read(os.path.join(here, "crates/store/src/migrate.rs"))
        start = text.find("MIGRATIONS")
        store_last = LABEL.findall(text[start:text.find("];", start)])[-1]
        tree = fresh()
        put(tree, "crates/store/src/migrate.rs", read(os.path.join(tree, "crates/store/src/migrate.rs")).replace(
            "];\n\npub(crate) const CURRENT_SCHEMA_VERSION",
            f"    (crate::x::migration_{store_last}_again, Older::Welcome),\n];\n\npub(crate) const CURRENT_SCHEMA_VERSION", 1))
        expect("a store label listed twice", tree, True, "listed twice")

        tree = fresh()
        put(tree, "crates/store/src/migrate.rs", read(os.path.join(tree, "crates/store/src/migrate.rs")).replace(
            "];\n\npub(crate) const CURRENT_SCHEMA_VERSION",
            "    (crate::x::migration_0003_back, Older::Welcome),\n];\n\npub(crate) const CURRENT_SCHEMA_VERSION", 1))
        expect("a store label going backwards", tree, True, "listed after")

        tree = fresh()
        put(tree, "crates/store/src/migrate.rs", "pub const MIGRATIONS: &[M] = &[\n    (migration_0001_a, Older::Refused),\n];\n")
        expect("a MIGRATIONS list the parse can't read", tree, True, "fewer than")

        tree = fresh()
        put(tree, "crates/store/src/x.rs", "pub fn migration_0042_other(c: &C) {}\n")
        expect("a second definition of store label 0042", tree, True, "0042")

        tree = fresh()
        put(tree, "crates/store/src/x.rs", "// fn migration_0042_other mentioned in a comment\n")
        expect("a store label in a comment", tree, False)

        site = read(os.path.join(here, "services/site/wrangler.toml"))
        relay_id = NAMESPACE.findall(read(os.path.join(here, "services/relay/wrangler.toml")))[0]
        site_id = NAMESPACE.findall(site)[0]
        tree = fresh()
        put(tree, "services/site/wrangler.toml", site.replace(f'namespace_id = "{site_id}"', f'namespace_id = "{relay_id}"', 1))
        expect("a site namespace the relay already holds", tree, True, f'"{relay_id}"')

        tree = fresh()
        put(tree, "services/site/wrangler.toml", site.replace(f'namespace_id = "{site_id}"', f'# namespace_id = "{relay_id}"\nnamespace_id = "{site_id}"', 1))
        expect("a commented-out namespace", tree, False)
    finally:
        shutil.rmtree(scratch, ignore_errors=True)
    if failures:
        print("migration-lint self-test FAILED:\n  " + "\n  ".join(failures), file=sys.stderr)
        return 1
    print("migration-lint self-test passed.")
    return 0


def main(argv):
    if argv == ["--self-test"]:
        return self_test()
    if argv:
        print("usage: migration-lint.py [--self-test]", file=sys.stderr)
        return 2
    code = run(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
    if code == 0:
        print("migration-lint: ok.")
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
