"""One real turn through `claude` in stream-json mode, recorded."""
import json, os, subprocess, tempfile, threading, time

env = {k: v for k, v in os.environ.items()
       if k not in ("CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CODE_SSE_PORT")}

# The flags `handshake::launch_args` sends, so the capture is what a pane
# actually sees. `--include-partial-messages` above all: without it the CLI
# sends no stream_event frames and the fixture would not contain the deltas the
# normalizer now reads the answer from.
args = ["claude", "--print",
        "--input-format", "stream-json",
        "--output-format", "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--permission-prompt-tool", "stdio",
        "--permission-mode", "bypassPermissions"]

p = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                     stderr=subprocess.PIPE, text=True, env=env,
                     cwd=tempfile.gettempdir())
assert p.stdin and p.stdout

lines, done = [], threading.Event()


def read():
    assert p.stdout
    for line in p.stdout:
        lines.append(line.rstrip("\n"))
        if '"type":"result"' in line.replace(" ", ""):
            done.set()
            return


threading.Thread(target=read, daemon=True).start()


def send(o):
    assert p.stdin
    p.stdin.write(json.dumps(o) + "\n")
    p.stdin.flush()


send({"type": "control_request", "request_id": "1", "request": {"subtype": "initialize"}})
time.sleep(6)
send({"type": "user",
      "message": {"role": "user", "content": [{"type": "text", "text": "Reply with exactly: hi"}]},
      "parent_tool_use_id": None})

done.wait(timeout=120)
time.sleep(0.5)
p.kill()

open("claude_turn.jsonl", "w").write("\n".join(lines) + "\n")
print("frames:", len(lines), "\n")
for line in lines:
    try:
        v = json.loads(line)
    except Exception:
        print("  RAW:", line[:100]); continue
    t = v.get("type")
    if t == "assistant":
        blocks = [b.get("type") for b in (v.get("message", {}).get("content") or [])]
        texts = [b.get("text", "")[:40] for b in (v.get("message", {}).get("content") or [])
                 if b.get("type") == "text"]
        print(f"  assistant blocks={blocks} text={texts} session={v.get('session_id','')[:8]}")
    elif t == "result":
        print(f"  result subtype={v.get('subtype')} keys={sorted(v.keys())[:12]}")
    elif t == "system":
        print(f"  system subtype={v.get('subtype')}")
    elif t == "control_response":
        print(f"  control_response id={(v.get('response') or {}).get('request_id')}")
    else:
        print(f"  {t}")
