#!/usr/bin/env python3
"""Run gate self-tests against source contents with synthetic Git history (ov-1305).

The index supplies filenames only: no real HEAD, commit, ref or history is read.
All gate plans, summaries, clones and commits belong to the temporary repository.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


def run(source):
    with tempfile.TemporaryDirectory(prefix="lane-gate-source-") as temporary:
        repo = Path(temporary)
        env = {k: v for k, v in os.environ.items() if not k.startswith("FARCOOLER_")}
        env.update(LANE_GATE_TEST_FIXTURE="1", GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
        names = subprocess.check_output(["git", "-C", str(source), "ls-files", "--cached", "--others",
                                         "--exclude-standard", "-z"], env=env).decode().split("\0")
        for name in names:
            if not name or not (source / name).is_file():
                continue
            target = repo / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source / name, target)
        def git(*args):
            subprocess.run(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                            "-c", "commit.gpgsign=false", *args], cwd=repo, env=env, check=True,
                           stdout=subprocess.DEVNULL)
        git("init", "-q")
        # Explicit paths, including ignored source inputs, never the caller's Git index.
        for start in range(0, len(names), 200):
            present = [n for n in names[start:start + 200] if n and (repo / n).is_file()]
            if present:
                git("add", "-f", "--", *present)
        git("commit", "-qm", "synthetic gate base")
        git("update-ref", "refs/remotes/origin/main", "HEAD")
        green = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, env=env, text=True).strip()
        candidates = repo / ".git/green.ndjson"
        candidates.write_text(json.dumps({"head_sha": green}) + "\n")
        env["CANDIDATES"] = str(candidates)
        return subprocess.call([sys.executable, str(repo / "scripts/lane-gate.py"), "--self-test"], cwd=repo, env=env)
