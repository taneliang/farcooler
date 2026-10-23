#!/usr/bin/env python3
"""Pick the simulator an iOS UI run boots (ov-710).

    xcrun simctl list devices available -j | ios-ui-pick-simulator.py iPhone
    ios-ui-pick-simulator.py --self-test

Prints one line, `<udid> <exact|fallback> <name> <runtime>`, and exits 1 with
nothing on stdout when the family has no device at all. The family is `iPhone`
or `iPad`, as `ios-ui-shards.py --device` says.

The wanted device is `iPhone 17` (every local timing and layout-sensitive
assertion was measured on it) or, for the iPad shard, an `iPad Pro 11-inch`. It
is taken from ANY installed runtime, newest first: a runner image whose newest
runtime has no iPhone 17 (iOS 27.1 carries only an iPhone Duo, 466 x 678 pt)
still has one on an older runtime. Only with no such device anywhere does it
fall back to the first device of the family on the newest runtime that has one.
"""
import json
import sys

WANT = {"iPhone": "iPhone 17", "iPad": "iPad Pro 11-inch"}


def version(runtime):
    tail = runtime.rsplit(".", 1)[-1]
    return [int(p) for p in tail.split("-")[1:] if p.isdigit()]


def label(runtime):
    return "iOS " + ".".join(str(n) for n in version(runtime))


def pick(devices, family):
    want = WANT[family]
    ios = sorted((r for r in devices if ".SimRuntime.iOS-" in r), key=version, reverse=True)
    for runtime in ios:
        for d in devices[runtime]:
            if d["name"] == want or (family == "iPad" and d["name"].startswith(want)):
                return d["udid"], "exact", d["name"], label(runtime)
    for runtime in ios:
        phones = [d for d in devices[runtime] if d["name"].startswith(family)]
        if phones:
            return phones[0]["udid"], "fallback", phones[0]["name"], label(runtime)
    return None


def device(udid, name):
    return {"udid": udid, "name": name, "isAvailable": True}


def self_test():
    rt = "com.apple.CoreSimulator.SimRuntime.iOS-"
    cases = [
        ("iPhone 17 on the newest runtime",
         {rt + "26-5": [device("A", "iPhone 17")], rt + "27-0": [device("B", "iPhone 17")]},
         "iPhone", ("B", "exact", "iPhone 17", "iOS 27.0")),
        ("iPhone 17 only on an older runtime, a Duo on the newest",
         {rt + "26-5": [device("A", "iPhone 17")], rt + "27-1": [device("D", "iPhone Duo")]},
         "iPhone", ("A", "exact", "iPhone 17", "iOS 26.5")),
        ("sorted by version, not as text",
         {rt + "9-0": [device("A", "iPhone 17")], rt + "27-1": [device("D", "iPhone 17")]},
         "iPhone", ("D", "exact", "iPhone 17", "iOS 27.1")),
        ("no iPhone 17 anywhere",
         {rt + "26-5": [device("A", "iPhone 16")], rt + "27-1": [device("D", "iPhone Duo")]},
         "iPhone", ("D", "fallback", "iPhone Duo", "iOS 27.1")),
        ("an iPhone 17e or Pro is not an iPhone 17",
         {rt + "27-0": [device("E", "iPhone 17e"), device("P", "iPhone 17 Pro")]},
         "iPhone", ("E", "fallback", "iPhone 17e", "iOS 27.0")),
        ("no iPhone at all",
         {rt + "27-0": [device("T", "iPad Pro 11-inch (M5)")], "com.apple.CoreSimulator.SimRuntime.watchOS-27-0": [device("W", "iPhone 17")]},
         "iPhone", None),
        ("the iPad shard takes the 11-inch Pro from an older runtime",
         {rt + "26-5": [device("T", "iPad Pro 11-inch (M5)")], rt + "27-0": [device("U", "iPad (A16)")]},
         "iPad", ("T", "exact", "iPad Pro 11-inch (M5)", "iOS 26.5")),
    ]
    bad = [f"{name}: got {pick(devs, fam)}, want {want}" for name, devs, fam, want in cases if pick(devs, fam) != want]
    for line in bad:
        print(f"ios-ui-pick-simulator self-test: {line}", file=sys.stderr)
    if not bad:
        print("ios-ui-pick-simulator self-test: ok")
    return 1 if bad else 0


def main(argv):
    if argv == ["--self-test"]:
        return self_test()
    if len(argv) != 1 or argv[0] not in WANT:
        print(__doc__, file=sys.stderr)
        return 2
    chosen = pick(json.load(sys.stdin)["devices"], argv[0])
    if chosen is None:
        return 1
    print(*chosen)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
