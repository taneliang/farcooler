#!/usr/bin/env python3
"""Is an environment ready for strict device-key matching? Read-only (ov-573).

Before the owner sets DEVICE_KEYS_ENFORCED = "1" (ruling R-72) on a relay, the
runbook (ov-550, with ov-565's additions) lists gates checked by hand. This
reports each one as ready, not ready or unknown, with the evidence, and
changes nothing:

    scripts/device-keys-readiness.py --env canary|preview|stable
    scripts/device-keys-readiness.py --env stable --deployed-at 1760000000000
    scripts/device-keys-readiness.py --self-test

It runs `wrangler d1 execute <db> --remote --json --command "SELECT ..."` and
nothing else: a query that is not a single SELECT is refused before wrangler is
called, and no token is read or printed (wrangler uses its own login, or
CLOUDFLARE_API_TOKEN from the environment). The wrangler command is `npx
wrangler`; set WRANGLER (for example `WRANGLER="pnpm exec wrangler"`) to change it.
It runs from services/relay so wrangler finds the same wrangler.toml a deploy
does.

THE GATES
  migrations   0025 (trusted_at), 0026 (sessions_revoked_at) and 0027 (device
               hold confirmations) are in the environment's d1_migrations. The
               names are read from services/relay/migrations, not repeated here.
               A migration says nothing about the worker: relay.yml applies them
               before it deploys, so the deploy itself is not checked.
  certify, hold, rekey
               The phones that reported in the last --days days (default 30) run
               a build at or past the commit that shipped it. Apps report
               "<display> · <build>" to the relay and the build is the commit
               count, so the floor is `git rev-list --count <commit>`; --min-build
               overrides it. A phone that reported no version is older than all of
               these.
  mac          Unknown always. The Mac updates through Sparkle and the relay keeps
               no record of its build.
  backfill     The device keys registered before the certifying apps shipped and
               not revoked, which no app has announced (runbook step 4). It counts
               every such key, reviewed or not, so it is ready only where there is
               none; a non-zero count is the list to review. Needs --deployed-at, the
               time of that deploy in milliseconds; without it, unknown, and
               the query to run is printed. Only a count is read; the rows hold
               emails, so the owner runs the printed query.

Exit status is 0 when every gate is ready, 1 otherwise, 2 for a usage error.
"""

import json
import os
import pathlib
import re
import shlex
import subprocess
import sys

try:
    import tomllib
except ImportError:  # Python before 3.11; the self-test does not need it
    tomllib = None

ROOT = pathlib.Path(__file__).resolve().parent.parent
RELAY = ROOT / "services" / "relay"
ENVS = ("canary", "preview", "stable")
READY, NOT_READY, UNKNOWN = "ready", "not ready", "unknown"

# The migrations the gates need, by number; the names come from the folder.
REQUIRED_MIGRATIONS = ("0025", "0026", "0027")

# The commits that shipped each half; the floor is their commit count.
SHIPPED_IN = {
    "certify": ("da89213d3", "ov-550"),
    "hold": ("ed49672e8", "ov-565"),
    "rekey": ("3e476474a", "ov-569"),
}
PLATFORMS = {"apns": "iPhone and iPad", "fcm": "Android"}


class Unreadable(Exception):
    """wrangler could not answer; the gates that needed it are unknown."""


# ---- querying, read-only -------------------------------------------------

FORBIDDEN = re.compile(
    r"\b(insert|update|delete|drop|alter|create|replace|pragma|attach|vacuum|reindex)\b", re.I
)


def check_read_only(sql):
    """A single SELECT, or refuse. The only door to wrangler goes through this."""
    text = sql.strip().rstrip(";").strip()
    if not re.match(r"select\b", text, re.I) or ";" in text or FORBIDDEN.search(text):
        raise ValueError(f"refusing a query that is not a single SELECT: {sql!r}")
    return text


def parse_rows(stdout):
    """The rows of `wrangler d1 execute --json`, which may follow a banner line."""
    start = next((i for i, line in enumerate(stdout.splitlines()) if line.lstrip().startswith(("[", "{"))), None)
    if start is None:
        raise Unreadable("wrangler printed no JSON")
    try:
        data = json.loads("\n".join(stdout.splitlines()[start:]))
    except json.JSONDecodeError as e:
        raise Unreadable(f"wrangler's JSON did not parse ({e.msg})")
    if isinstance(data, dict):
        data = [data]
    rows = []
    for part in data:
        if not isinstance(part, dict) or part.get("success") is False:
            raise Unreadable("wrangler reported the query failed")
        rows.extend(part.get("results") or [])
    return rows


def scrub(text):
    """No token-shaped string reaches the terminal, whatever wrangler echoes."""
    return re.sub(r"[A-Za-z0-9_\-]{32,}", "<redacted>", text)


def failure_line(output, code):
    """Wrangler ends on its log-file banner, so name its last error line, not its last line."""
    lines = [x.strip() for x in output.splitlines() if x.strip()]
    errors = [x for x in lines if x.startswith("\u2718") or "[ERROR]" in x]
    rest = [x for x in lines if "Logs were written to" not in x]
    return (errors or rest or [f"exit {code}"])[-1]


def d1_rows(target, sql):
    cmd = shlex.split(os.environ.get("WRANGLER", "npx wrangler")) + ["d1", "execute", target["db"], "--remote", "--json"]
    cmd += target["flag"] + ["--command", check_read_only(sql)]
    try:
        done = subprocess.run(cmd, cwd=RELAY, capture_output=True, text=True, timeout=120,
                              stdin=subprocess.DEVNULL)
    except (OSError, subprocess.TimeoutExpired) as e:
        raise Unreadable(f"could not run wrangler ({type(e).__name__})")
    if done.returncode != 0:
        raise Unreadable("wrangler failed: " + scrub(failure_line(done.stderr or done.stdout, done.returncode))[:200])
    return parse_rows(done.stdout)


# ---- the tree --------------------------------------------------------------


def target_for(env, config):
    """The database name and wrangler flag for an environment; stable is the absence of --env."""
    block = config if env == "stable" else config.get("env", {}).get(env, {})
    dbs = block.get("d1_databases") or []
    if not dbs:
        raise Unreadable(f"wrangler.toml names no database for {env}")
    return {"db": dbs[0]["database_name"], "flag": [] if env == "stable" else ["--env", env]}


def flag_state(env, config):
    block = config if env == "stable" else config.get("env", {}).get(env, {})
    return (block.get("vars") or {}).get("DEVICE_KEYS_ENFORCED")


def required_migration_names(names):
    found = []
    for number in REQUIRED_MIGRATIONS:
        match = sorted(n for n in names if n.startswith(number + "_"))
        if len(match) != 1:
            return None
        found.append(match[0])
    return found


def git_count(commit):
    """The commit count, or None: a shallow clone counts a plausible wrong number (see version.sh)."""
    try:
        shallow = subprocess.run(["git", "rev-parse", "--is-shallow-repository"], cwd=ROOT, capture_output=True,
                                 text=True, timeout=30, stdin=subprocess.DEVNULL)
        if shallow.returncode != 0 or shallow.stdout.strip() != "false":
            return None
        out = subprocess.run(["git", "rev-list", "--count", commit], cwd=ROOT, capture_output=True, text=True,
                             timeout=30, stdin=subprocess.DEVNULL)
        return int(out.stdout.strip()) if out.returncode == 0 else None
    except (OSError, ValueError, subprocess.TimeoutExpired):
        return None


# ---- the gates -------------------------------------------------------------


def migrations_gate(required, applied):
    if required is None:
        return UNKNOWN, ["the migrations folder no longer has exactly one file for each of " + ", ".join(REQUIRED_MIGRATIONS)], []
    missing = [n for n in required if n not in applied]
    evidence = [f"{n}: {'applied' if n in applied else 'NOT applied'}" for n in required]
    return (NOT_READY if missing else READY), evidence, []


def build_of(version):
    """The build a phone reported as "<display> · <build>"; None when it has none."""
    match = re.search(r"·\s*(\d+)\s*$", version or "")
    return int(match.group(1)) if match else None


def adoption_gate(rows, floor, days):
    """rows: [{platform, version, n}] for phones heard from in the window."""
    first = f"floor: build {floor}" if floor is not None else "floor: unknown"
    if floor is None:
        return UNKNOWN, [first, "no usable commit count here (shallow or missing history); pass --min-build"], []
    if not rows:
        return UNKNOWN, [first, f"no phone reported in the last {days} days"], []
    below, unparsed, evidence = 0, 0, []
    for row in sorted(rows, key=lambda r: (r["platform"], r["version"] or "")):
        build = build_of(row["version"])
        if row["version"] is None:
            verdict, below = "reported no version, so older than the floor", below + row["n"]
        elif build is None:
            verdict, unparsed = "build not readable", unparsed + row["n"]
        elif build < floor:
            verdict, below = "below the floor", below + row["n"]
        else:
            verdict = "at or past the floor"
        who = PLATFORMS.get(row["platform"], row["platform"])
        evidence.append(f"{who}: {row['version'] or '(none)'} x{row['n']} ({verdict})")
    evidence.insert(0, first)
    if below:
        return NOT_READY, evidence, []
    if unparsed:
        return UNKNOWN, evidence, []
    return READY, evidence, []


def backfill_gate(deployed_at, count):
    query = (
        "SELECT a.email, d.label, d.state, d.created_at FROM device_keys d JOIN accounts a "
        f"ON a.id = d.account_id WHERE d.state <> 'revoked' AND d.created_at < {deployed_at if deployed_at is not None else '<deploy ms>'} ORDER BY d.created_at"
    )
    if deployed_at is None:
        return UNKNOWN, ["needs --deployed-at, the certifying apps' deploy in milliseconds"], [query]
    if count is None:
        return UNKNOWN, ["the count could not be read"], [query]
    evidence = [f"{count} device key(s) registered before that deploy and not revoked (any state but revoked, reviewed or not)"]
    return (NOT_READY if count else READY), evidence, [query] if count else []


def mac_gate(floors=None):
    shown = ", ".join(f"{k} {v if v is not None else 'unknown'}" for k, v in (floors or {}).items())
    return UNKNOWN, ["manual: the relay keeps no record of the Mac's build, so this gate is always unknown"], [
        "Check the stable appcast's newest build against these floors: " + (shown or "see the phone gates")
    ]


def gather(env, deployed_at, days, min_builds, now_ms, config=None, names=None):
    """Every gate for an environment as (name, verdict, evidence, hints)."""
    if config is None:
        config = tomllib.loads((RELAY / "wrangler.toml").read_text()) if tomllib else {}
    if names is None:
        names = [p.name for p in (RELAY / "migrations").glob("*.sql")]
    gates = []
    try:
        target = target_for(env, config)
    except Unreadable as e:
        target, why = None, str(e)
    else:
        why = None

    def ask(sql):
        if target is None:
            raise Unreadable(why)
        return d1_rows(target, sql)

    def run(name, thunk, lead=()):
        try:
            verdict, evidence, hints = thunk()
        except Unreadable as e:
            verdict, evidence, hints = UNKNOWN, [*lead, str(e)], []
        gates.append((name, verdict, evidence, hints))

    required = required_migration_names(names)
    run("Relay migrations 0025 to 0027 applied", lambda: migrations_gate(
        required, {r["name"] for r in ask("SELECT name FROM d1_migrations")}))

    since = now_ms - days * 86400 * 1000
    phone_rows = {}

    def phones():
        if "rows" not in phone_rows:
            phone_rows["rows"] = ask(
                f"SELECT platform, version, COUNT(*) AS n FROM devices WHERE updated_at >= {int(since)} GROUP BY platform, version")
        return phone_rows["rows"]

    floors = {}
    for key, label in (("certify", "Certifying apps adopted"), ("hold", "Trust-reset hold adopted"), ("rekey", "Phone re-key adopted")):
        commit, what = SHIPPED_IN[key]
        floor = min_builds.get(key) if min_builds.get(key) is not None else git_count(commit)
        floors[key] = floor
        run(f"{label} on phones, from {what}", lambda floor=floor: adoption_gate(phones(), floor, days),
            lead=[f"floor: build {floor}" if floor is not None else "floor: unknown"])

    run("Mac builds adopted", lambda: mac_gate(floors))

    def backfill():
        if deployed_at is None:
            return backfill_gate(None, None)
        rows = ask(f"SELECT COUNT(*) AS n FROM device_keys WHERE state <> 'revoked' AND created_at < {int(deployed_at)}")
        return backfill_gate(deployed_at, rows[0]["n"] if rows else None)

    run("Backfill of devices never announced", backfill)
    return gates, flag_state(env, config)


def render(env, gates, flag):
    out = [f"Device-key readiness: {env}", f"DEVICE_KEYS_ENFORCED in wrangler.toml: {flag if flag is not None else 'unset (strict matching is off)'}", ""]
    for name, verdict, evidence, hints in gates:
        out.append(f"[{verdict.upper()}] {name}")
        out += [f"    {line}" for line in evidence]
        out += [f"    run: {line}" for line in hints]
    verdicts = [g[1] for g in gates]
    out += ["", "Ready to flip: " + ("yes" if all(v == READY for v in verdicts) else
            f"no ({verdicts.count(NOT_READY)} not ready, {verdicts.count(UNKNOWN)} unknown)")]
    return "\n".join(out)


# ---- self-test -------------------------------------------------------------


def self_test():
    failures = []

    def check(name, got, want):
        if got != want:
            failures.append(f"{name}: got {got!r}, wanted {want!r}")

    names = ["0024_x.sql", "0025_device_trusted_at.sql", "0026_sessions_revoked_at.sql",
             "0027_device_hold_confirmations.sql", "0028_other.sql"]
    required = required_migration_names(names)
    check("required migration names", required, names[1:4])
    check("a missing file makes the folder unreadable", required_migration_names(names[:3]), None)
    allm = set(names)
    check("migrations all applied", migrations_gate(required, allm)[0], READY)
    check("migrations missing 0027", migrations_gate(required, allm - {names[3]})[0], NOT_READY)
    check("migrations missing 0027 names it", [l for l in migrations_gate(required, allm - {names[3]})[1] if "NOT" in l],
          ["0027_device_hold_confirmations.sql: NOT applied"])
    check("migrations with an unreadable folder", migrations_gate(None, allm)[0], UNKNOWN)

    # wrangler's output: a banner, then the JSON
    sample = ' ⛅️ wrangler 3.100\n[\n  {"results": [{"name": "0025_a.sql"}, {"name": "0026_b.sql"}], "success": true}\n]\n'
    check("parse rows after a banner", parse_rows(sample), [{"name": "0025_a.sql"}, {"name": "0026_b.sql"}])
    for label, text in (("no JSON", "Not logged in\n"), ("bad JSON", "[ {"), ("a failed query", '[{"success": false, "results": []}]')):
        try:
            parse_rows(text)
            failures.append(f"{label}: parse_rows should raise Unreadable")
        except Unreadable:
            pass

    def row(platform, version, n=1):
        return {"platform": platform, "version": version, "n": n}

    check("build read from a reported version", build_of("0.2.0 (canary) · 3400"), 3400)
    check("no build in a bare version", build_of("0.2.0"), None)
    check("adoption: all past the floor", adoption_gate([row("apns", "0.2.0 · 3400"), row("fcm", "0.2.0 · 3332", 2)], 3332, 30)[0], READY)
    check("adoption: one phone below", adoption_gate([row("apns", "0.2.0 · 3400"), row("fcm", "0.1.9 · 3331")], 3332, 30)[0], NOT_READY)
    check("adoption: a phone with no version", adoption_gate([row("apns", None)], 3332, 30)[0], NOT_READY)
    check("adoption: nobody reported", adoption_gate([], 3332, 30)[0], UNKNOWN)
    check("adoption: a build that can't be read", adoption_gate([row("apns", "0.2.0 · 3400"), row("fcm", "odd")], 3332, 30)[0], UNKNOWN)
    check("adoption: below beats unreadable", adoption_gate([row("apns", "0.1 · 10"), row("fcm", "odd")], 3332, 30)[0], NOT_READY)
    check("adoption: no floor", adoption_gate([row("apns", "0.2.0 · 3400")], None, 30)[0], UNKNOWN)

    check("backfill: no deploy time", backfill_gate(None, None)[0], UNKNOWN)
    check("backfill: none unannounced", backfill_gate(1000, 0)[0], READY)
    check("backfill: some unannounced", backfill_gate(1000, 4)[0], NOT_READY)
    check("backfill: the query leaves out revoked keys", "state <> 'revoked'" in backfill_gate(1000, 1)[2][0], True)
    check("backfill: the evidence says what it counts", "not revoked" in backfill_gate(1000, 1)[1][0], True)
    check("adoption: floor shown when unknown", adoption_gate([], 3332, 30)[1][0], "floor: build 3332")
    check("adoption: no floor shown as unknown", adoption_gate([], None, 30)[1][0], "floor: unknown")
    check("backfill: count unreadable", backfill_gate(1000, None)[0], UNKNOWN)
    check("mac is always unknown", mac_gate({"certify": 5})[0], UNKNOWN)

    # The read-only guard
    for sql in ("SELECT name FROM d1_migrations", "select count(*) as n from devices;"):
        try:
            check_read_only(sql)
        except ValueError:
            failures.append(f"guard refused a SELECT: {sql}")
    for sql in ("UPDATE devices SET version = 'x'", "SELECT 1; DELETE FROM devices", "DROP TABLE devices",
                "INSERT INTO devices VALUES (1)", "SELECT 1 FROM x WHERE a = 1; ALTER TABLE x ADD COLUMN y", "PRAGMA table_info(x)",
                "SELECT name FROM d1_migrations WHERE 0 OR (DELETE FROM devices)"):
        try:
            check_read_only(sql)
            failures.append(f"guard let through: {sql}")
        except ValueError:
            pass
    check("scrub hides a token-shaped string", scrub("bad token cfut_" + "a" * 40), "bad token <redacted>")

    banner = '\U0001FAB5  Logs were written to "/Users/x/.wrangler/logs/wrangler.log"'
    error = "\u2718 [ERROR] Failed to fetch auth token: 400 Bad Request"
    check("failure line skips the log banner", failure_line(f"{error}\n\n{banner}\n", 1), error)
    check("failure line takes the last error", failure_line(f"\u2718 [ERROR] a\nnote\n[ERROR] b\n{banner}", 1), "[ERROR] b")
    check("failure line without an error skips the banner", failure_line(f"plain failure\n{banner}\n", 1), "plain failure")
    check("failure line with only the banner is the exit code", failure_line(banner, 7), "exit 7")
    check("failure line with no output is the exit code", failure_line("", 7), "exit 7")

    # The whole run, on canned answers and no network
    seen = []

    def canned(applied, phones, count):
        answers = {"d1_migrations": [{"name": n} for n in applied], "FROM devices": phones, "FROM device_keys": [{"n": count}]}

        def fake(target, sql):
            seen.append(sql)
            check_read_only(sql)  # the queries gather really builds must pass the guard
            return next(v for k, v in answers.items() if k in sql)
        return fake

    config = {"d1_databases": [{"database_name": "farcooler"}], "env": {
        "canary": {"d1_databases": [{"database_name": "farcooler-canary"}], "vars": {"DEVICE_KEYS_ENFORCED": "1"}},
        "preview": {"d1_databases": [{"database_name": "farcooler-preview"}]}}}
    global d1_rows
    real = d1_rows
    try:
        d1_rows = canned(names, [row("apns", "0.3 · 9000"), row("fcm", "0.3 · 9000")], 0)
        gates, flag = gather("canary", 1000, 30, {"certify": 1, "hold": 1, "rekey": 1}, 10**12, config, names)
        check("end to end: the Mac keeps it from being ready", [g[1] for g in gates],
              [READY, READY, READY, READY, UNKNOWN, READY])
        check("end to end: the flag is read per environment", flag, "1")
        check("end to end: the backfill count leaves out revoked keys",
              [q for q in seen if "FROM device_keys" in q and "revoked" in q] != [], True)
        check("end to end: stable's flag is unset", gather("stable", None, 30, {"certify": 1, "hold": 1, "rekey": 1}, 10**12, config, names)[1], None)
        d1_rows = canned(names[:3], [row("apns", "0.1 · 5")], 3)
        gates, _ = gather("canary", 1000, 30, {"certify": 100, "hold": 100, "rekey": 100}, 10**12, config, names)
        check("end to end: stale environment", [g[1] for g in gates],
              [NOT_READY, NOT_READY, NOT_READY, NOT_READY, UNKNOWN, NOT_READY])

        def broken(target, sql):
            raise Unreadable("wrangler failed: not logged in")
        d1_rows = broken
        gates, _ = gather("preview", None, 30, {"certify": 1, "hold": 1, "rekey": 1}, 10**12, config, names)
        check("end to end: wrangler down is unknown, never ready", {g[1] for g in gates}, {UNKNOWN})
        check("end to end: wrangler down still shows the floor", gates[1][2][0], "floor: build 1")
    finally:
        d1_rows = real
    # d1_rows itself: it refuses a write before wrangler runs, and wrangler runs with no stdin
    calls = []
    real_run = subprocess.run

    def fake_run(cmd, **kw):
        calls.append((cmd, kw))
        return subprocess.CompletedProcess(cmd, 0, '[{"results": [{"n": 1}], "success": true}]', "")
    subprocess.run = fake_run
    try:
        try:
            d1_rows({"db": "x", "flag": []}, "DELETE FROM devices")
            failures.append("d1_rows ran a DELETE")
        except ValueError:
            pass
        check("d1_rows ran nothing for a DELETE", calls, [])
        check("d1_rows answers a SELECT", d1_rows({"db": "x", "flag": ["--env", "canary"]}, "SELECT 1 AS n"), [{"n": 1}])
        subprocess.run = lambda cmd, **kw: subprocess.CompletedProcess(
            cmd, 1, "", f"\u2718 [ERROR] bad token cfut_{'a' * 40}\n\n{banner}\n")
        try:
            d1_rows({"db": "x", "flag": []}, "SELECT 1")
            failures.append("d1_rows hid a wrangler failure")
        except Unreadable as e:
            check("d1_rows names the error and scrubs it", str(e), "wrangler failed: \u2718 [ERROR] bad token <redacted>")
        subprocess.run = fake_run
        check("d1_rows runs wrangler without stdin", calls[0][1].get("stdin"), subprocess.DEVNULL)
        check("d1_rows passes the environment flag", "--env" in calls[0][0] and "--remote" in calls[0][0], True)
        def git_says(shallow, count="3332\n"):
            def run_git(cmd, **kw):
                out = shallow if "--is-shallow-repository" in cmd else count
                return subprocess.CompletedProcess(cmd, 0, out, "")
            subprocess.run = run_git
            return git_count("abc")
        check("git_count: a full clone", git_says("false\n"), 3332)
        check("git_count: a shallow clone has no floor", git_says("true\n"), None)
    finally:
        subprocess.run = real_run
    text = render("canary", gates, None)
    check("the report says it is not ready to flip", "Ready to flip: no" in text, True)

    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1
    print("device-keys-readiness self-test: ok")
    return 0


def main(argv):
    if argv == ["--self-test"]:
        return self_test()
    args = {"env": None, "deployed-at": None, "days": "30", "min-build": []}
    it = iter(argv)
    for a in it:
        name = a[2:] if a.startswith("--") else None
        if name not in args:
            print(__doc__.split("\n\n")[1], file=sys.stderr)
            return 2
        try:
            value = next(it)
        except StopIteration:
            print(f"{a} needs a value", file=sys.stderr)
            return 2
        if name == "min-build":
            args[name].append(value)
        else:
            args[name] = value
    if args["env"] not in ENVS:
        print("usage: device-keys-readiness.py --env canary|preview|stable [--deployed-at MS] [--days N] [--min-build certify=N]", file=sys.stderr)
        return 2
    min_builds = {}
    for item in args["min-build"]:
        key, _, value = item.partition("=")
        if key not in SHIPPED_IN or not value.isdigit():
            print("--min-build takes certify=N, hold=N or rekey=N", file=sys.stderr)
            return 2
        min_builds[key] = int(value)
    if tomllib is None:
        print("this needs Python 3.11 or newer to read wrangler.toml", file=sys.stderr)
        return 2
    import time
    try:
        deployed_at = int(args["deployed-at"]) if args["deployed-at"] else None
        days = int(args["days"])
    except ValueError:
        print("--days and --deployed-at take whole numbers (--deployed-at in milliseconds)", file=sys.stderr)
        return 2
    gates, flag = gather(args["env"], deployed_at, days, min_builds, int(time.time() * 1000))
    print(render(args["env"], gates, flag))
    return 0 if all(g[1] == READY for g in gates) else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
