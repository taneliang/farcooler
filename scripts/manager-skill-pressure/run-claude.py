#!/usr/bin/env python3
"""run-claude.py <S> <label> <n>: one pressure run, end to end (ov-323).

Builds the world, renders the skill WITHOUT cargo (the same substitution as
skill_install::body, so keep it in step), runs `claude -p --model sonnet` in a
sandbox HOME and scores it. Worlds go to $FC_RUNS (default /tmp/fc-t/skill-runs),
whose `home` is the sandbox HOME and CLAUDE_CONFIG_DIR; SKILL_SRC names another
SKILL.md to test. Appends one JSON line to $FC_RUNS/results.jsonl.
Supports S13, S16, S18 and S19; add a prompt to PROMPTS for another."""
import json, os, pathlib, subprocess, sys
S, ver, n = sys.argv[1:4]
WT = str(pathlib.Path(__file__).resolve().parents[2])
H = f"{WT}/scripts/manager-skill-pressure"
ROOT = os.environ.get("FC_RUNS", "/tmp/fc-t/skill-runs")
d = f"{ROOT}/{ver}-{S}-{n}"
PROMPTS = {
 "S13": "You started a subagent in this session on fc-2: its launch result said agentId a7f3c91e. Now: fc-5 goes first and fc-2 second, and fc-4 is third. fc-6 is for after the next release. fc-7 is parked: nobody plans to do it.",
 "S16": "fc-3's agent wants to know whether the empty inbox says 'Nothing here' or 'All caught up'. I don't care which, don't wait on me for calls like that.",
 "S18": "integ-3 is reviewed and its gates passed. Land it.",
 "S19": "Do a check-in.",
}
subprocess.run([f"{H}/new-scratch-repo.sh", S, d, "--baseline"], check=True, stdout=subprocess.DEVNULL)
d = str(pathlib.Path(d).resolve())
# render without cargo: same substitution as skill_install::body + shell_quote
src = open(os.environ.get("SKILL_SRC") or f"{WT}/crates/daemon/assets/manager/SKILL.md").read()
rs = open(f"{WT}/crates/daemon/src/skill_install.rs").read()
import re
wait = ("Stop. Nothing will wake you: Far Cooler can't yet type into this pane when the board changes. End your turn by telling the owner in one line what you're waiting on, and that you'll look again when they next talk to you. Don't poll the board in a loop, don't sleep, and don't say you'll check back.")
fm = ("---\nname: manager\ndescription: Manage a Far Cooler workspace's task board as its orchestrator. Reads the charter, reads the board, then creates, revises, answers or reports. Never does the work itself. Use only when the owner asks for the manager.\ndisable-model-invocation: true\n---\n")
q = "'" + (d + "/farcooler").replace("'", "'\\''") + "'"
skill = src.replace("{{frontmatter}}", fm).replace("{{wait}}", wait).replace("{{cli}}", q)
assert "{{" not in skill
open(f"{d}/skill.md", "w").write(skill)
prompt = (f"You are this workspace's orchestrator. The repository is {d}/repo. Your shell doesn't carry the pane's environment: start every shell command with `. {d}/pane.env &&`. Follow this skill exactly:\n\n{skill}\n\n---\n\n{PROMPTS[S]}")
env = dict(os.environ, HOME=f"{ROOT}/home", CLAUDE_CONFIG_DIR=f"{ROOT}/home/.claude")
env["PATH"] = f"{d}/bin:" + env["PATH"]  # the pane has the fake gh on PATH in every command, not only after pane.env
for k in list(env):
    if k.startswith("CLAUDE_CODE") or k in ("CLAUDECODE", "CLAUDE_PID", "AI_AGENT", "CLAUDE_EFFORT"):
        env.pop(k)
p = subprocess.Popen(["claude", "-p", "--model", "sonnet", "--output-format", "json", "--dangerously-skip-permissions", prompt],
                     cwd=f"{d}/home", env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
open(f"{d}/pid", "w").write(str(p.pid))
try:
    out, err = p.communicate(timeout=900)
except subprocess.TimeoutExpired:
    os.kill(p.pid, 15); out, err = p.communicate(); err += " TIMEOUT"
try:
    j = json.loads(out)
except Exception:
    j = {"result": out}
open(f"{d}/reply.txt", "w").write(j.get("result") or "")
sc = subprocess.run([f"{H}/score.py", S, d], capture_output=True, text=True)
open(f"{d}/score.txt", "w").write(sc.stdout + sc.stderr)
fails = [l for l in sc.stdout.splitlines() if l.startswith("FAIL")]
rec = {"S": S, "ver": ver, "run": n, "pass": sc.returncode == 0, "fails": fails, "cost": j.get("total_cost_usd"), "turns": j.get("num_turns"), "model": list((j.get("modelUsage") or {}).keys()), "err": err[-300:]}
print(json.dumps(rec))
open(f"{ROOT}/results.jsonl", "a").write(json.dumps(rec) + "\n")
