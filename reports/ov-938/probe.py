#!/usr/bin/python3
"""Private kernel receipt: no discovered PIDs or external group signals."""
import os,subprocess,time,signal,json,ctypes
p=subprocess.Popen(['/usr/bin/python3','-c','import os,time; child=os.fork(); print(os.getpid(),os.getpgrp(),child,flush=True) if child else None; time.sleep(10)'],stdout=subprocess.PIPE,text=True,preexec_fn=os.setpgrp)
ids=p.stdout.readline().strip()
os.killpg(p.pid,signal.SIGKILL)
receipts=[]
for n in range(100):
    if n==10: p.wait(timeout=2)
    try: os.killpg(p.pid,0); result='exists'
    except OSError as e: result=e.errno
    receipts.append([n,result])
    if result==3: break
    time.sleep(.01)
p.wait(timeout=2)
record={'ids':ids,'leader_reaped':p.returncode,'group_probe':receipts}
leader=subprocess.Popen(['/usr/bin/python3','-c','pass'],preexec_fn=os.setpgrp)
libc=ctypes.CDLL(None,use_errno=True)
assert libc.waitid(1,leader.pid,ctypes.create_string_buffer(128),4|32)==0
try: os.killpg(leader.pid,signal.SIGKILL); signaled=True
except OSError as e: signaled=e.errno
status=leader.wait(timeout=2)
try: os.killpg(leader.pid,0); gone=False
except OSError as e: gone=e.errno
record['leader_only']={'pid':leader.pid,'signal':signaled,'reaped_status':status,'group_probe_after_reap':gone}
print(json.dumps(record))
