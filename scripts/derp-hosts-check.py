#!/usr/bin/env python3
"""Keep the web client's DERP_HOSTS equal to the hosts in Tailcat's DERP map.

The web shell's CSP names Tailcat's relays exactly, not as *.ipn.dev (ov-764
review), so a relay added to https://tailcat.dev/derpmap.json that is not in
DERP_HOSTS (services/web/wrangler.toml) makes every connect to a runner homed
there fail with a false "didn't answer". This reads the map and either

    scripts/derp-hosts-check.py              # fail, with a diff, on a difference
    scripts/derp-hosts-check.py --file MAP   # read a saved map, not the network
    scripts/derp-hosts-check.py --self-test

A daily CI job (.github/workflows/derp-hosts.yml) runs the check and opens or
updates one issue on a difference. Exit 1 is drift, in either direction: a host
in the map and not in DERP_HOSTS breaks connects; one in DERP_HOSTS and not in
the map is trust nobody needs. Exit 2 is a map or a wrangler.toml this script
cannot read, loud on purpose: a layout change must never read as "no drift".
"""
import contextlib
import io
import json
import re
import sys
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WRANGLER = ROOT / "services/web/wrangler.toml"
URL = "https://tailcat.dev/derpmap.json"


class Unreadable(Exception):
    pass


def map_hosts(text):
    """Every node HostName in a DERP map, lowercased."""
    try:
        regions = json.loads(text)["Regions"]
        hosts = {n["HostName"].strip().lower() for r in regions.values() for n in r["Nodes"]}
    except (ValueError, KeyError, TypeError, AttributeError) as e:
        raise Unreadable(f"the DERP map is not the shape this script reads: {e!r}")
    hosts.discard("")
    if not hosts:
        raise Unreadable("the DERP map lists no hosts")
    return hosts


def configured_hosts(toml):
    """{section: set of hosts} for every DERP_HOSTS line, by its [section]."""
    out, section = {}, "default"
    for line in toml.splitlines():
        head = re.match(r"\s*\[([^\]]+)\]\s*$", line)
        if head:
            section = head.group(1)
            continue
        m = re.match(r'\s*DERP_HOSTS\s*=\s*"([^"]*)"', line)
        if m:
            out[section] = set(m.group(1).lower().split())
    if not out:
        raise Unreadable("no DERP_HOSTS line in wrangler.toml")
    return out


def drift(hosts, configured):
    """Lines describing each difference; empty when there is none."""
    lines = []
    for env, have in sorted(configured.items()):
        for h in sorted(hosts - have):
            lines.append(f"{h} is in the DERP map but not in DERP_HOSTS ({env})")
        for h in sorted(have - hosts):
            lines.append(f"{h} is in DERP_HOSTS ({env}) but not in the DERP map")
    return lines


def check(map_text, toml):
    return drift(map_hosts(map_text), configured_hosts(toml))


def fetch():
    req = urllib.request.Request(URL, headers={"User-Agent": "farcooler-derp-hosts-check"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return r.read().decode()


def self_test():
    def dmap(*hs):
        return json.dumps({"Regions": {str(i): {"Nodes": [{"HostName": h}]} for i, h in enumerate(hs)}})

    toml = 'DERP_HOSTS = "a.example b.example"\n[env.canary.vars]\nDERP_HOSTS = "a.example b.example"\n'
    fails = []

    def expect(name, got, want):
        if got != want:
            fails.append(f"{name}: got {got!r}, want {want!r}")

    expect("equal sets", check(dmap("a.example", "B.example"), toml), [])
    expect("new relay in the map", check(dmap("a.example", "b.example", "c.example"), toml),
           ["c.example is in the DERP map but not in DERP_HOSTS (default)",
            "c.example is in the DERP map but not in DERP_HOSTS (env.canary.vars)"])
    expect("relay gone from the map", check(dmap("a.example"), toml),
           ["b.example is in DERP_HOSTS (default) but not in the DERP map",
            "b.example is in DERP_HOSTS (env.canary.vars) but not in the DERP map"])
    one_stale = 'DERP_HOSTS = "a.example b.example"\n[env.canary.vars]\nDERP_HOSTS = "a.example"\n'
    expect("one environment behind", check(dmap("a.example", "b.example"), one_stale),
           ["b.example is in the DERP map but not in DERP_HOSTS (env.canary.vars)"])
    for name, bad in (("empty regions", '{"Regions": {}}'), ("not json", "<html>"), ("no Regions", "{}")):
        try:
            map_hosts(bad)
            fails.append(f"{name}: read as a map")
        except Unreadable:
            pass
    try:
        configured_hosts("name = 'x'\n")
        fails.append("a wrangler.toml without DERP_HOSTS read as fine")
    except Unreadable:
        pass
    expect("the real wrangler.toml has both environments",
           sorted(configured_hosts(WRANGLER.read_text())), ["env.canary.vars", "vars"])
    with tempfile.NamedTemporaryFile("w", suffix=".json") as f:
        f.write(dmap("a.example", "zz.example"))
        f.flush()
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            code = run(Path(f.name).read_text(), toml)
        expect("a saved map with drift exits 1", code, 1)
    for f in fails:
        print("self-test FAIL:", f, file=sys.stderr)
    if not fails:
        print("self-test ok")
    return 1 if fails else 0


def run(map_text, toml):
    try:
        lines = check(map_text, toml)
    except Unreadable as e:
        print(f"cannot check: {e}", file=sys.stderr)
        return 2
    for line in lines:
        print(line)
    if lines:
        print("Fix: add the host to DERP_HOSTS in every environment of services/web/wrangler.toml "
              "(see services/web/README.md).", file=sys.stderr)
    return 1 if lines else 0


def main(argv):
    if argv == ["--self-test"]:
        return self_test()
    if argv and not (argv[0] == "--file" and len(argv) == 2):
        print(__doc__, file=sys.stderr)
        return 2
    try:
        map_text = Path(argv[1]).read_text() if argv else fetch()
        toml = WRANGLER.read_text()
    except (OSError, ValueError) as e:
        print(f"cannot check: {e}", file=sys.stderr)
        return 2
    return run(map_text, toml)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
