#!/usr/bin/env python3
"""Drive the ACP adapter directly and capture every frame it sends.

Sends the same `initialize` Far Cooler sends, creates a session, then prompts
for work that forces a Task dispatch. Answers fs/* and permission requests so
the turn can actually finish.

Usage: acp_spike.py <out.jsonl> <worktree> [--no-subagent-cap]
"""
import json
import os
import subprocess
import sys
import threading
import time

out_path, worktree = sys.argv[1], sys.argv[2]
want_subagent = "--no-subagent-cap" not in sys.argv

PROMPT = os.environ.get("SPIKE_PROMPT") or (
    "Use the Task tool to dispatch a subagent (subagent_type general-purpose) "
    "that reads main.rs in this directory and reports how many lines it has. "
    "Do not read the file yourself — dispatch the subagent and report what it "
    "returns. Keep it short."
)

log = open(out_path, "w")
lock = threading.Lock()


def record(direction, obj):
    with lock:
        log.write(json.dumps({"dir": direction, "frame": obj}) + "\n")
        log.flush()


env = dict(os.environ)
claude = os.path.expanduser("~/.local/bin/claude")
if os.path.exists(claude):
    env["CLAUDE_CODE_EXECUTABLE"] = claude

proc = subprocess.Popen(
    ["npx", "-y", "@agentclientprotocol/claude-agent-acp"],
    cwd=worktree,
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.DEVNULL,
    env=env,
    text=True,
    bufsize=1,
)

next_id = [1]
responses = {}
done = threading.Event()


def send(obj):
    record("out", obj)
    proc.stdin.write(json.dumps(obj) + "\n")
    proc.stdin.flush()


def request(method, params):
    rid = next_id[0]
    next_id[0] += 1
    send({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})
    return rid


def wait_for(rid, timeout=120):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if rid in responses:
            return responses.pop(rid)
        time.sleep(0.05)
    raise TimeoutError(f"no response to {rid}")


def reader():
    for line in proc.stdout:
        line = line.strip()
        if not line:
            continue
        try:
            frame = json.loads(line)
        except json.JSONDecodeError:
            record("in-malformed", line)
            continue
        record("in", frame)

        # Adapter-initiated requests must be answered or the turn hangs.
        if frame.get("method") and frame.get("id") is not None:
            method, params = frame["method"], frame.get("params") or {}
            if method == "fs/read_text_file":
                try:
                    content = open(params["path"]).read()
                except OSError:
                    content = ""
                send({"jsonrpc": "2.0", "id": frame["id"], "result": {"content": content}})
            elif method == "fs/write_text_file":
                try:
                    with open(params["path"], "w") as f:
                        f.write(params.get("content", ""))
                except OSError:
                    pass
                send({"jsonrpc": "2.0", "id": frame["id"], "result": {}})
            elif method == "session/request_permission":
                opts = params.get("options") or []
                pick = next(
                    (o for o in opts if "allow" in o.get("kind", "")),
                    opts[0] if opts else None,
                )
                send({
                    "jsonrpc": "2.0",
                    "id": frame["id"],
                    "result": {"outcome": {"outcome": "selected", "optionId": pick["optionId"]}}
                    if pick else {"outcome": {"outcome": "cancelled"}},
                })
            else:
                send({"jsonrpc": "2.0", "id": frame["id"], "result": {}})
        elif frame.get("id") is not None:
            responses[frame["id"]] = frame
    done.set()


threading.Thread(target=reader, daemon=True).start()

caps = {"fs": {"readTextFile": True, "writeTextFile": True}}
if want_subagent:
    caps["_meta"] = {"subagent-transcript": True}

rid = request("initialize", {"protocolVersion": 1, "clientCapabilities": caps})
init = wait_for(rid, 90)
print("initialize ok", file=sys.stderr)

rid = request("session/new", {"cwd": worktree, "mcpServers": []})
new = wait_for(rid, 90)
session_id = new["result"]["sessionId"]
print(f"session {session_id}", file=sys.stderr)

rid = request(
    "session/prompt",
    {"sessionId": session_id, "prompt": [{"type": "text", "text": PROMPT}]},
)
try:
    result = wait_for(rid, 300)
    print(f"stopReason={result.get('result', {}).get('stopReason')}", file=sys.stderr)
except TimeoutError:
    print("prompt timed out", file=sys.stderr)

proc.terminate()
log.close()
print(f"wrote {out_path}", file=sys.stderr)
