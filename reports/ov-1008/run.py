#!/usr/bin/env python3
"""Run three alternating Release comparisons on a dedicated, booted simulator.

python3 reports/ov-1008/run.py <simulator-udid> <Release-iphonesimulator/FarCooler.app> <output.json>
Build using the command in README.md first. No accessibility queries run during
measurement. Each launch starts a new process and waits for FrameWatch's file.
"""
import json
import subprocess
import sys
import time
from pathlib import Path


def sim(*args, check=True):
    return subprocess.run(["xcrun", "simctl", *args], check=check, capture_output=True, text=True).stdout.strip()


def run(device, app, output):
    bundle = "com.farcooler.spike.ov1008"
    sim("install", device, app)
    records = []
    for mode in ["SwiftUI", "UIKit", "UIKit", "SwiftUI", "SwiftUI", "UIKit"]:
        sim("terminate", device, bundle, check=False)
        container = Path(sim("get_app_container", device, bundle, "data"))
        report = container / "Documents/fling.txt"
        report.unlink(missing_ok=True)
        args = ["-appLock", "NO", "-native-agent-harness", "-native-live", "-native-live-real", "-native-long-reports", "-frame-watch"]
        if mode == "UIKit":
            args.append("-uikit-transcript")
        sim("launch", device, bundle, *args)
        deadline = time.monotonic() + 90
        while not report.exists() and time.monotonic() < deadline:
            time.sleep(1)
        if not report.exists():
            raise RuntimeError(f"{mode}: no FrameWatch report")
        raw = report.read_text()
        fields = dict(word.split("=", 1) for word in raw.split() if "=" in word)
        assert fields.get("container") == mode, raw
        assert int(fields.get("rows-start", 0)) >= 400, raw
        assert int(fields.get("script-distance", 0)) >= 30_000, raw
        assert float(fields.get("seconds", 0)) >= 19, raw
        record = {"mode": mode, "raw": raw}
        records.append(record)
        Path(output).write_text(json.dumps({"device": device, "configuration": "Release -O + DEBUG fixtures", "runs": records}, indent=2) + "\n")
        print(mode, raw, flush=True)
    sim("terminate", device, bundle)


if __name__ == "__main__":
    run(*sys.argv[1:])
