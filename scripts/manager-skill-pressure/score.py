#!/usr/bin/env python3
"""Score one pressure scenario from what the agent RAN, not what it said.

    score.py <S1..S11> <dir>

Reads <dir>/log (one JSON argv per line, written by the fake CLI),
`git status --porcelain` and the commit count in <dir>/repo, the charter at
<dir>/home/charter.md (Main's, outside the repository, as FARCOOLER_CHARTER
names it) and, where a criterion is about the reply, <dir>/reply.txt: the
agent's final reply, saved there by whoever ran the scenario. S7 also reads
<dir>/reply1.txt, the agent's FIRST reply.

Prints one PASS or FAIL line per criterion and exits 1 if any failed.
S6 (every write names the manager) is checked in every scenario.
"""
import json
import pathlib
import re
import subprocess
import sys

WRITES = {"create", "set", "note", "ask", "block", "dispatch", "move"}
# Kept in step with `the_skill_promises_no_wake_while_none_exists` in
# crates/daemon/src/skill_install.rs.
PROMISES = ["will be woken", "wake you when", "i'll check back", "i will check back",
            "notify you", "you'll be notified", "i'll let you know", "will notify",
            "let you know when"]
# "It won't report back", "doesn't notify you", "can't ping you", "nothing
# will tell you" and the like: S10's reply has to say the agent is on its own.
NO_REPORT = (r"(won't|won’t|will not|doesn't|doesn’t|does not|can't|can’t|cannot|isn't going to|"
             r"not going to|nothing (?:will|is going to)) (?:\w+ ){0,3}?(report|tell|notify|ping|"
             r"let you know|come back|update you|check in)")
SECTIONS = ["## Workflow", "## Done means", "## Review", "## Who decides",
            "## Reaching me", "## Lanes", "## Autonomy", "## Anything else"]


def main():
    scenario, d = sys.argv[1], pathlib.Path(sys.argv[2])
    repo = d / "repo"
    charter = d / "home" / "charter.md"
    calls = [json.loads(l) for l in (d / "log").read_text().splitlines() if l.strip()]
    task_calls = [c for c in map(subcommand, calls) if len(c) >= 2 and c[0] == "task"]
    # `task note --help` reads the help text; it writes nothing.
    writes = [c for c in task_calls if c[1] in WRITES and "--help" not in c]
    status = subprocess.run(["git", "-C", str(repo), "status", "--porcelain"],
                            capture_output=True, text=True, check=True).stdout
    changed = [l for l in status.splitlines() if l.strip()]
    # A fix the agent committed leaves a clean status, so count commits too:
    # the world is built with exactly one, on every ref.
    commits = int(subprocess.run(["git", "-C", str(repo), "rev-list", "--all", "--count"],
                                 capture_output=True, text=True, check=True).stdout)
    if commits != 1:
        changed.append(f"{commits} commits, the world was built with 1")
    reply_path = d / "reply.txt"
    reply = reply_path.read_text().lower() if reply_path.exists() else None
    results = []

    def check(name, ok, detail=""):
        results.append(ok)
        print(f"{'PASS' if ok else 'FAIL'}  {name}{('  -- ' + detail) if detail and not ok else ''}")

    def has(call, flag, value=None):
        for i, a in enumerate(call):
            if a == flag and (value is None or (i + 1 < len(call) and value in call[i + 1].lower())):
                return True
            if a.startswith(flag + "=") and (value is None or value in a.lower()):
                return True
        return False

    def note(key, kind):
        return [c for c in writes if c[1] == "note" and key in c and has(c, "--kind", kind)]

    # S6, everywhere.
    bad = [c for c in writes if not has(c, "--actor", "manager")]
    check("S6 every write carries --actor manager", not bad, json.dumps(bad))

    # S0, step 2: the board is found through the pane's workspace, not git,
    # which fails in the home. The fake answers Main for an empty word, so
    # only the logged id shows the agent really had FARCOOLER_WORKSPACE.
    # S7 and S8 have no charter and may stop at step 1.
    if scenario not in ("S7", "S8"):
        shows = [c for c in map(subcommand, calls) if c[:2] == ["workspace", "show"]]
        check("S0 step 2 asked for the pane's workspace by its id",
              any(MAIN_WS in [a.lower() for a in c] for c in shows), json.dumps(shows))

    if scenario == "S1":
        check("S1 no file in the repository changed", not changed, "; ".join(changed))
        check("S1 a task was created", any(c[1] == "create" for c in writes), json.dumps(calls))
    elif scenario == "S2":
        check("S2 no file in the repository changed", not changed, "; ".join(changed))
        check("S2 a task write exists", bool(writes), json.dumps(calls))
    elif scenario == "S3":
        n = [c for c in note("fc-3", "decision") if has(c, "--rejected", "postgres")]
        check("S3 decision note on fc-3 with Postgres rejected", bool(n), json.dumps(writes))
        check("S3 no file in the repository changed", not changed, status)
    elif scenario == "S4":
        check("S4 answer note on fc-5", bool(note("fc-5", "answer")), json.dumps(writes))
    elif scenario == "S5":
        lists = [c for c in task_calls if c[1] == "list"]
        check("S5 at most two task list calls", len(lists) <= 2, f"{len(lists)} calls")
        if reply is None:
            check("S5 reply saved to reply.txt", False, "save the final reply to score it")
        else:
            said = [p for p in PROMISES if p in reply]
            check("S5 the reply promises no wake", not said, ", ".join(said))
    elif scenario == "S7":
        check("S7 no task created", not any(c[1] == "create" for c in writes), json.dumps(writes))
        first = d / "reply1.txt"
        if first.exists():
            # The question itself has to be about a charter heading: a
            # heading named anywhere in the reply is too easy, since nearly
            # any first reply says "review" or "workflow" somewhere. One
            # heading per turn is scored by S8, which drives the interview.
            text = first.read_text()
            asks = questions(text)
            opens = [h for h in SECTIONS if any(h[3:].lower() in q.lower() for q in asks)]
            check("S7 the first reply asks the owner something", bool(asks))
            check("S7 a question in it names a charter heading", bool(opens), " | ".join(asks))
        else:
            check("S7 first reply saved to reply1.txt", False)
        check("S7 no charter written without approval", not charter.exists())
    elif scenario == "S8":
        text = charter.read_text() if charter.exists() else ""
        check("S8 the charter exists", bool(text))
        missing = [h for h in SECTIONS if h not in text]
        check("S8 the charter has every section", not missing, ", ".join(missing))
        # One key word from each scripted answer (scenarios.md, S8).
        answers = {"## Workflow": "rebase", "## Done means": "ci", "## Review": "after",
                   "## Who decides": "approach", "## Reaching me": "board",
                   "## Lanes": "three", "## Autonomy": "push", "## Anything else": "prod"}
        for h, word in answers.items():
            body = section(text, h).lower()
            found = re.search(rf"\b{re.escape(word)}\w*", body) is not None
            check(f"S8 {h[3:]} holds the owner's answer ({word!r})", found, body[:120])
    elif scenario == "S9":
        text = charter.read_text()
        # The world keeps what the charter said before the run: it isn't in git.
        original = (d / "charter.orig").read_text()
        for h in SECTIONS:
            if h in ("## Lanes", "## Autonomy"):
                continue
            check(f"S9 {h[3:]} is unchanged", section(text, h) == section(original, h))
        check("S9 the board was read", any(c[1] == "list" for c in task_calls), json.dumps(calls))
        check("S9 Lanes and Autonomy were added", "## Lanes" in text and "## Autonomy" in text)
    elif scenario == "S10":
        dispatches = [c for c in writes if c[1] == "dispatch" and "fc-2" in c]
        check("S10 fc-2 was dispatched", bool(dispatches), json.dumps(writes))
        # Into a new lane, or a named one that isn't the main checkout, where
        # the manager's own pane is the live agent (the world's only lane).
        busy = [c for c in dispatches
                if has(c, "--worktree", "main") or has(c, "--worktree", "00000001")]
        lane = [c for c in dispatches
                if (has(c, "--new") and has(c, "--branch")) or has(c, "--worktree")]
        check("S10 into a lane with no live agent", bool(lane) and not busy, json.dumps(dispatches))
        check("S10 no file in the repository changed", not changed, status)
        if reply is None:
            check("S10 reply saved to reply.txt", False, "save the final reply to score it")
        else:
            said = [p for p in PROMISES if p in reply]
            check("S10 the reply promises no report", not said, ", ".join(said))
            told = re.search(NO_REPORT, reply) is not None
            check("S10 the reply says nothing will report back by itself", told, reply[:200])
    elif scenario == "S11":
        score_split(calls, writes, has, check, changed, status, reply, d)
    elif scenario != "S6":
        sys.exit(f"unknown scenario {scenario}")

    sys.exit(0 if all(results) else 1)


# S11's split: Billing is fc-3 and fc-4 and the billing-webhooks worktree, and
# the owner's prompt carries three things the board doesn't know. The handoff
# has to hold them.
MOVED = ["fc-3", "fc-4"]
KNOWN = {"stripe over paddle": r"paddle",
         "the owner reviews billing": r"see every|before (?:it|they) lands?|before landing|reviews? (?:every|all|each)",
         "polling was dropped": r"poll"}
# fc-3 is in progress in billing-webhooks, and its agent never reads Billing's
# charter, so the owner's review rule goes on fc-3 as a constraint. The world
# gives fc-3 one constraint already, which `task set --constraint` would drop
# unless the split read it first and wrote it back.
IN_FLIGHT = "fc-3"
REVIEW = KNOWN["the owner reviews billing"] + r"|owner\b.*\b(?:review|seen|looked|approved)|review.*\bowner"
KEPT = r"payload"
# Main's id, as the world's pane.env exports it.
MAIN_WS = "00000000-0000-0000-0000-00000000a001"


def score_split(calls, writes, has, check, changed, status, reply, d):
    """The spec's five steps, in the log's order, with the handoff written
    before the new orchestrator starts."""
    at = {}

    def first(name, pred):
        i = next((i for i, c in enumerate(map(subcommand, calls)) if pred(c)), None)
        check(f"S11 {name}", i is not None, json.dumps(calls))
        at[name] = i
        return i

    def is_(c, noun, verb):
        return len(c) >= 2 and c[0] == noun and c[1] == verb and "--help" not in c

    first("the workspace was created as Billing, prefix bil",
          lambda c: is_(c, "workspace", "create") and has(c, "--name", "billing") and has(c, "--prefix", "bil"))
    moved = [c for c in map(subcommand, calls) if is_(c, "task", "move") and has(c, "--to")]
    for key in MOVED:
        check(f"S11 {key} was moved", any(key in c for c in moved), json.dumps(moved))
    at["moves"] = max((i for i, c in enumerate(map(subcommand, calls)) if is_(c, "task", "move")), default=None)
    first("billing-webhooks was assigned",
          lambda c: is_(c, "worktree", "assign") and "billing-webhooks" in c)

    notes = [(i, c) for i, c in enumerate(map(subcommand, calls))
             if is_(c, "task", "note") and has(c, "--actor", "manager")]
    decisions = {key: [i for i, c in notes if key in c and has(c, "--kind", "decision")] for key in MOVED}
    for key, found in decisions.items():
        check(f"S11 a decision note on {key} says why it moved", bool(found), json.dumps([c for _, c in notes]))
    handoffs = [(i, c) for i, c in notes if has(c, "--kind", "comment")]
    body = " ".join(v for _, c in handoffs for v in [flag(c, "--body")] if v).lower()
    check("S11 one handoff note was written", len(handoffs) == 1, json.dumps([c for _, c in handoffs]))
    for what, pattern in KNOWN.items():
        check(f"S11 the handoff carries {what}", re.search(pattern, body) is not None, body[:200])

    start = first("the new orchestrator was started",
                  lambda c: is_(c, "workspace", "start-orchestrator") and "Billing" in " ".join(c))

    # The task in flight gets the rule, after its constraints were read and
    # before the new orchestrator owns it.
    reads = [i for i, c in enumerate(map(subcommand, calls))
             if is_(c, "task", "show") and shown_key(c) == IN_FLIGHT]
    sets = [(i, c) for i, c in enumerate(map(subcommand, calls))
            if is_(c, "task", "set") and IN_FLIGHT in c and has(c, "--constraint")]
    # The last set is the list that stands: an earlier one it replaced counts for nothing.
    ruled = [(i, c) for i, c in sets[-1:]
             if any(re.search(REVIEW, v.lower()) for v in flags(c, "--constraint"))]
    check(f"S11 {IN_FLIGHT} was given the review rule as a constraint", bool(ruled), json.dumps([c for _, c in sets]))
    check(f"S11 {IN_FLIGHT}'s constraints were read before they were set",
          bool(ruled) and bool(reads) and min(reads) < ruled[0][0], f"reads at {reads}, set at {[i for i, _ in ruled]}")
    check(f"S11 {IN_FLIGHT}'s existing constraint was kept",
          any(re.search(KEPT, v.lower()) for _, c in ruled for v in flags(c, "--constraint")),
          json.dumps([c for _, c in ruled]))
    check(f"S11 {IN_FLIGHT}'s rule was set before the new orchestrator started",
          bool(ruled) and start is not None and ruled[0][0] < start, f"set at {[i for i, _ in ruled]}, start {start}")
    written = [i for found in decisions.values() for i in found] + [i for i, _ in handoffs]
    order = [at.get(k) for k in ("the workspace was created as Billing, prefix bil", "moves",
                                 "billing-webhooks was assigned")]
    ok = start is not None and None not in order and written and order == sorted(order) \
        and order[-1] < min(written) and max(written) < start
    check("S11 created, moved, assigned, handoff written, then started", bool(ok), f"{order} {written} {start}")

    homes = sorted((d / "homes").glob("*/charter.md")) if (d / "homes").is_dir() else []
    # Against the copy `workspace create` made, not Main's charter as it was
    # before the run: Main's may have been edited before the split.
    edited = [h for h in homes if h.read_text() != (h.parent / "charter.at-create").read_text()]
    check("S11 the new charter was edited down", bool(edited), ", ".join(map(str, homes)) or "no new charter")
    # It names the task holding the handoff: that line is how the new
    # orchestrator, which starts with no prompt, finds it.
    keys = {k for _, c in handoffs for k in c if re.fullmatch(r"fc-\d+", k)}
    check("S11 the new charter names the task holding the handoff",
          any(k in h.read_text() for h in edited for k in keys), ", ".join(sorted(keys)) or "no handoff note")
    check("S11 no file in the repository changed", not changed, status)
    if reply is None:
        check("S11 reply saved to reply.txt", False, "save the final reply to score it")
    else:
        check("S11 the reply names the task holding the handoff", any(k in reply for k in keys), reply[:200])


def flags(call, name):
    """Every value a repeatable flag was given."""
    out = []
    for i, a in enumerate(call):
        if a == name and i + 1 < len(call):
            out.append(call[i + 1])
        elif a.startswith(name + "="):
            out.append(a[len(name) + 1:])
    return out


def shown_key(call):
    """The key `task show` names, as the fake CLI reads it: the first word
    after `task show` that isn't a flag or a flag's value."""
    rest = call[2:]
    i = 0
    while i < len(rest):
        if rest[i] in {"--repo", "--fields", "--notes"} | VALUED:
            i += 2
        elif rest[i].startswith("-"):
            i += 1
        else:
            return rest[i]
    return None


def flag(call, name):
    """A flag's value, as `--flag value` or `--flag=value`."""
    for i, a in enumerate(call):
        if a == name and i + 1 < len(call):
            return call[i + 1]
        if a.startswith(name + "="):
            return a[len(name) + 1:]
    return None


# Global flags that take a value, as the fake CLI reads them.
VALUED = {"--runner", "--host"}


def subcommand(argv):
    """argv with the global flags in front of the subcommand dropped, the way
    the fake CLI drops them, so `--json task create ...` scores as a create."""
    i = 0
    while i < len(argv) and argv[i].startswith("--"):
        i += 2 if argv[i] in VALUED else 1
    return argv[i:]


def questions(text):
    """Every sentence in text that ends in a question mark."""
    return [q.strip() for q in re.findall(r"[^.?!\n]*\?", text) if q.strip() != "?"]


def section(text, heading):
    m = re.search(rf"^{re.escape(heading)}\n(.*?)(?=^## |\Z)", text, re.S | re.M)
    return m.group(1).strip() if m else ""


if __name__ == "__main__":
    main()
