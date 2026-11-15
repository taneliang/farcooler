#!/usr/bin/env python3
"""Make crates/workflow/tests/fixtures/migrate-snapshot.json from a live engine snapshot.

    python3 -I reports/ov-1281/sanitize.py SNAPSHOT.json OUT.json

Only the old `State` is kept (no configuration, notices or places). Every string is replaced by a stable
stand-in unless it is a step id, an enum variant name, a group id, a template source or a 40-hex commit (which becomes another
40-hex value), so equal strings stay equal and nothing private survives: no paths, briefs, failures, prompts,
argv, repository names or user names. Ticket keys become t-N. The subset keeps every live group with its
members, every ticket that is not Failed or at rest, and a few of each other (phase, step) class.
"""
import hashlib, json, re, sys
from collections import defaultdict

SALT = b"ov-1281"
VARIANT = re.compile(r"^[A-Z][A-Za-z]*$")
GROUP = re.compile(r"^group-\d+-[a-z]+$")
SHA = re.compile(r"^[0-9a-f]{40}$")
TICKET = re.compile(r"^[a-z]+-\d+$")
# The engine's own template sources, which the old reducer reads (`Definition::is_ticket`).
SOURCES = re.compile(r"^(ticket|tracker|coding|workflow)/v\d+$")


def h(s, n=12):
    return hashlib.sha256(SALT + s.encode()).hexdigest()[:n]


def main(src, out):
    state = json.load(open(src))["state"]
    items, archive = state["items"], state["archive"]
    steps = {st["id"] for i in items.values() for st in i["definition"]["stages"]} | {"landing"}
    keys = sorted(k for k in set(items) | set(archive) if TICKET.match(k))
    tickets = {k: f"t-{n}" for n, k in enumerate(keys)}

    def string(s):
        if s in tickets: return tickets[s]
        if s in steps or VARIANT.match(s) or GROUP.match(s) or SOURCES.match(s) or s == "": return s
        if SHA.match(s): return hashlib.sha1(SALT + s.encode()).hexdigest()
        return "x-" + h(s)

    def scrub(v):
        if isinstance(v, dict): return {string(k) if k in tickets else k: scrub(x) for k, x in v.items()}
        if isinstance(v, list): return [scrub(x) for x in v]
        if isinstance(v, str): return string(v)
        return v

    def step(i):
        st = i["definition"]["stages"]
        return st[i["stage"]]["id"] if i["stage"] < len(st) else "end"

    keep = set()
    for k, i in items.items():
        if i.get("members"):
            keep.add(k)
            keep.update(m["item"] for m in i["members"])
    classes = defaultdict(list)
    for k, i in sorted(items.items()):
        tracked = all(s["kind"] == "Tracker" for s in i["definition"]["stages"])
        if i["phase"] in ("Failed", "WaitingApproval") or tracked or i.get("group_of"):
            classes[(i["phase"], step(i), bool(i.get("group_of")))].append(k)
        else:
            keep.add(k)
    for ks in classes.values(): keep.update(ks[:3])
    kept_archive = sorted(archive)[-30:]
    out_state = dict(state)
    out_state["items"] = {string(k): scrub(items[k]) for k in sorted(keep)}
    out_state["archive"] = {string(k): scrub(archive[k]) for k in kept_archive}
    out_state["seen"] = [string(s) for s in state.get("seen", [])][-20:]
    out_state["source_sequences"] = {"x-" + h(k): v for k, v in state.get("source_sequences", {}).items()}
    out_state["workspace"] = "x-" + h(state["workspace"])
    text = json.dumps(out_state, indent=None, sort_keys=True, separators=(",", ":"))
    for word in ("/Users", "e-liang", "taneliang", "farcooler", "ov-", "@"):
        assert word not in text, f"{word!r} survived"
    open(out, "w").write(text + "\n")
    print(f"{len(out_state['items'])} items, {len(out_state['archive'])} archived, {len(text)} bytes")


if __name__ == "__main__":
    main(*sys.argv[1:])
