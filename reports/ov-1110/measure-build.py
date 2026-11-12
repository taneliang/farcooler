import os, subprocess, time, json, pathlib, threading
root=pathlib.Path.cwd(); out=root/'reports/ov-1110'; stop=threading.Event(); samples=[]
def monitor():
 while not stop.wait(2): samples.append([time.time(),*os.getloadavg()])
threading.Thread(target=monitor,daemon=True).start()
env={**os.environ,'CARGO_BUILD_JOBS':'3','CARGO_PROFILE_DEV_DEBUG':'0','CARGO_TARGET_DIR':str(root/'target/ov1110')}
results=[]
for label,cmd in [('cold',['cargo','build','--workspace','--locked','--offline','--timings']),('warm',['cargo','build','--workspace','--locked','--offline','--timings'])]:
 while max(os.getloadavg())>=20: time.sleep(15)
 start=time.time()
 with open(out/(label+'.log'),'w') as log: rc=subprocess.call(cmd,env=env,stdout=log,stderr=subprocess.STDOUT)
 results.append(dict(label=label,command=cmd,seconds=time.time()-start,exit=rc,load_start=[s for s in samples if s[0]>=start][:1],load_end=os.getloadavg()))
 timing=root/'target/ov1110/cargo-timings/cargo-timing.html'
 if timing.exists(): (out/(label+'-timing.html')).write_bytes(timing.read_bytes())
 if rc: break
stop.set(); (out/'build-results.json').write_text(json.dumps({'env':{k:env[k] for k in ['CARGO_BUILD_JOBS','CARGO_PROFILE_DEV_DEBUG','CARGO_TARGET_DIR']},'runs':results,'load_samples':samples},indent=2))
