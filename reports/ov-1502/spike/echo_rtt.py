"""Keystroke-to-echo over SSH without a pty: `ssh -T host cat` with pipes.
Writes one byte, times until the remote `cat` sends it back through the whole
path (ssh client -> transport -> sshd -> cat -> sshd -> transport -> client).
Usage: echo_rtt.py N -- ssh args..."""
import os, sys, time, json, statistics, subprocess, random, string, select
n=int(sys.argv[1]); cmd=sys.argv[sys.argv.index('--')+1:]
t0=time.monotonic()
p=subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, bufsize=0)
fdout=p.stdout.fileno()
def rt(c, timeout=30):
    t=time.perf_counter(); p.stdin.write(c)
    if not select.select([fdout],[],[],timeout)[0]: raise TimeoutError
    got=os.read(fdout,64); assert c in got, got
    return (time.perf_counter()-t)*1000
rt(b"Z", 90); setup=time.monotonic()-t0
for _ in range(3): rt(b"w")   # warm-up
s=[]
for _ in range(n):
    s.append(rt(random.choice(string.ascii_lowercase).encode())); time.sleep(0.02)
p.stdin.close(); p.kill()
s.sort()
print(json.dumps({"n":n,"session_setup_s":round(setup,2),"median_ms":round(statistics.median(s),2),"p90_ms":round(s[int(.9*n)-1],2),"min_ms":round(s[0],2),"max_ms":round(s[-1],2)}))
