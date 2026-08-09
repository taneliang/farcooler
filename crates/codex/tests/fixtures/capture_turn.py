import json, subprocess, tempfile, threading, time

p = subprocess.Popen(["codex", "app-server"], stdin=subprocess.PIPE,
                     stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                     text=True, cwd=tempfile.gettempdir())
assert p.stdin and p.stdout

lines = []
done = threading.Event()

def read():
    assert p.stdout
    for line in p.stdout:
        lines.append(line.rstrip("\n"))
        if '"turn/completed"' in line:
            done.set()
            return

threading.Thread(target=read, daemon=True).start()

def send(obj):
    assert p.stdin
    p.stdin.write(json.dumps(obj) + "\n")
    p.stdin.flush()

send({"id": 1, "method": "initialize",
      "params": {"clientInfo": {"name": "farcooler", "version": "0.1.0"}}})
time.sleep(1.5)
send({"method": "initialized"})
send({"id": 2, "method": "thread/start", "params": {"cwd": tempfile.gettempdir()}})
time.sleep(3)

thread_id = None
for line in lines:
    try:
        v = json.loads(line)
    except Exception:
        continue
    if v.get("id") == 2 and "result" in v:
        thread_id = (v["result"].get("thread") or {}).get("id")
        print("thread obj:", json.dumps(v["result"].get("thread"))[:300])
        print("thread/start result keys:", sorted(v["result"].keys()))
print("threadId:", thread_id)

if thread_id:
    send({"id": 3, "method": "turn/start", "params": {
        "threadId": thread_id,
        "input": [{"type": "text", "text": "Reply with exactly: hi"}]}})
    done.wait(timeout=90)

time.sleep(1)
p.kill()
open("codex_turn.jsonl","w").write("\n".join(lines) + "\n")

print("\n=== method sequence ===")
for line in lines:
    try:
        v = json.loads(line)
    except Exception:
        continue
    m = v.get("method")
    if m:
        extra = ""
        params = v.get("params") or {}
        if "item" in params and isinstance(params["item"], dict):
            extra = f'  item.type={params["item"].get("type")}'
        print(f"  {m}{extra}")
    elif "id" in v:
        print(f"  <response id={v['id']}>")

print("\n=== a completed item, in full ===")
for line in lines:
    try:
        v = json.loads(line)
    except Exception:
        continue
    if v.get("method") == "item/completed":
        print(json.dumps(v["params"], indent=1)[:1200])
        break

print("\n=== turn/completed ===")
for line in lines:
    try:
        v = json.loads(line)
    except Exception:
        continue
    if v.get("method") == "turn/completed":
        print(json.dumps(v["params"], indent=1)[:800])
